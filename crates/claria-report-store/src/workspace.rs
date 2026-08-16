//! Workspace persistence: optimistic-concurrency loads and saves, plus
//! the session-level operations that mutate the accepted draft.

use aws_sdk_s3::Client as S3Client;
use claria_core::models::{
    client::Client,
    report::{
        AuthorshipKind, ReportContent, ReportExport, ReportExportStatus, ReportProposalDecision,
        ReportProposalResolution, ReportTemplateImport, ReportWorkspace, SectionAuthorship,
        decode_report_workspace,
    },
};
use claria_storage::error::StorageError;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    ACTIVE_RUN_MESSAGE, MAX_RESOLUTIONS, ReportExportSnapshot, ReportStoreError,
    ReportTemplateApplication,
};

/// Legacy-compatible current workspace loader. New UI sessions use
/// [`start_report_workspace`] and resume by ID with
/// [`load_report_workspace_by_id`].
pub async fn load_report_workspace(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<ReportWorkspace, ReportStoreError> {
    Ok(load_or_create(s3, bucket, client_id).await?.workspace)
}

/// Start an independent Writing session. Unlike the legacy singleton, every
/// new session gets its own object and can later be resumed from Editor
/// History without replacing another report.
pub async fn start_report_workspace(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<ReportWorkspace, ReportStoreError> {
    start_report_workspace_with_id(s3, bucket, client_id, Uuid::new_v4()).await
}

/// Idempotently start a session with a frontend-generated ID. React may replay
/// a mount effect in development; retrying the same start ID must return the
/// same empty session rather than creating duplicate Editor History rows.
pub async fn start_report_workspace_with_id(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<ReportWorkspace, ReportStoreError> {
    if client_id.is_nil() || report_id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "Client and report IDs must not be nil.".to_string(),
        ));
    }
    ensure_client_exists(s3, bucket, client_id).await?;
    let key = claria_core::s3_keys::report_session_workspace(client_id, report_id);
    match load_existing(s3, bucket, &key, client_id).await {
        Ok(loaded) if loaded.workspace.report_id == report_id => return Ok(loaded.workspace),
        Ok(_) => {
            return Err(ReportStoreError::InvalidWorkspace(
                "The Writing session ID does not match its storage key.".to_string(),
            ));
        }
        Err(LoadWorkspaceError::NotFound) => {}
        Err(error) => return Err(error.into_public()),
    }

    let existing = list_report_workspaces(s3, bucket, client_id).await?;
    let visible_session_count = existing
        .iter()
        .filter(|workspace| {
            workspace.draft.revision > 0
                || !workspace.session.turns.is_empty()
                || workspace.template_import.is_some()
        })
        .count();
    let now = jiff::Timestamp::now();
    let mut workspace = ReportWorkspace::new(client_id, now);
    workspace.report_id = report_id;
    workspace
        .rename_session(
            &format!("Writer Session ({})", visible_session_count + 1),
            now,
        )
        .map_err(|error| ReportStoreError::InvalidWorkspace(error.to_string()))?;
    match claria_storage::state::save_state_if_none_match(s3, bucket, &key, &workspace).await {
        Ok(_) => Ok(workspace),
        Err(StorageError::PreconditionFailed { .. }) => load_existing(s3, bucket, &key, client_id)
            .await
            .map(|loaded| loaded.workspace)
            .map_err(LoadWorkspaceError::into_public),
        Err(source) => Err(ReportStoreError::storage(
            "creating the Writing session",
            source,
        )),
    }
}

pub async fn load_report_workspace_by_id(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<ReportWorkspace, ReportStoreError> {
    if report_id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "Report ID must not be nil.".to_string(),
        ));
    }
    ensure_client_exists(s3, bucket, client_id).await?;
    Ok(load_for_report(s3, bucket, client_id, report_id)
        .await?
        .workspace)
}

/// List every independently resumable Writing session without creating one.
pub async fn list_report_workspaces(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<Vec<ReportWorkspace>, ReportStoreError> {
    if client_id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "Client ID must not be nil.".to_string(),
        ));
    }
    ensure_client_exists(s3, bucket, client_id).await?;

    let mut keys = claria_storage::objects::list_objects(
        s3,
        bucket,
        &claria_core::s3_keys::report_sessions_prefix(client_id),
    )
    .await
    .map_err(|source| ReportStoreError::storage("listing Writing sessions", source))?;
    let legacy_key = claria_core::s3_keys::report_workspace(client_id);
    match claria_storage::objects::get_object(s3, bucket, &legacy_key).await {
        Ok(_) => keys.push(legacy_key),
        Err(StorageError::NotFound { .. }) => {}
        Err(source) => {
            return Err(ReportStoreError::storage(
                "checking the legacy Writing session",
                source,
            ));
        }
    }

    use futures::StreamExt;
    let loads = keys.into_iter().map(|key| async move {
        load_existing(s3, bucket, &key, client_id)
            .await
            .map(|loaded| loaded.workspace)
            .map_err(LoadWorkspaceError::into_public)
    });
    let mut workspaces = futures::stream::iter(loads)
        .buffered(claria_storage::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    workspaces.sort_by_key(|workspace| std::cmp::Reverse(workspace.updated_at));
    Ok(workspaces)
}

/// Load the most recently updated existing session without creating one.
pub async fn find_report_workspace(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<Option<ReportWorkspace>, ReportStoreError> {
    Ok(list_report_workspaces(s3, bucket, client_id)
        .await?
        .into_iter()
        .next())
}

pub async fn rename_report_session(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    name: &str,
) -> Result<ReportWorkspace, ReportStoreError> {
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    loaded
        .workspace
        .rename_session(name, jiff::Timestamp::now())
        .map_err(|error| ReportStoreError::InvalidInput(error.to_string()))?;
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded.workspace)
}

pub async fn save_report_draft(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    expected_revision: u64,
    content: ReportContent,
) -> Result<ReportWorkspace, ReportStoreError> {
    let loaded = load_or_create(s3, bucket, client_id).await?;
    save_report_draft_loaded(s3, bucket, loaded, expected_revision, content).await
}

pub async fn save_report_draft_for_report(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    content: ReportContent,
) -> Result<ReportWorkspace, ReportStoreError> {
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    save_report_draft_loaded(s3, bucket, loaded, expected_revision, content).await
}

async fn save_report_draft_loaded(
    s3: &S3Client,
    bucket: &str,
    mut loaded: LoadedWorkspace,
    expected_revision: u64,
    mut content: ReportContent,
) -> Result<ReportWorkspace, ReportStoreError> {
    ensure_revision(&loaded.workspace, expected_revision)?;
    ensure_no_active_run(&loaded.workspace)?;
    if loaded.workspace.session.pending_proposal.is_some() {
        return Err(ReportStoreError::InvalidInput(
            "Accept or reject the pending proposal before editing the report.".to_string(),
        ));
    }
    let now = jiff::Timestamp::now();
    let saved_revision = expected_revision
        .checked_add(1)
        .ok_or_else(|| ReportStoreError::InvalidInput("Report revision overflow.".to_string()))?;
    // A hand edit arrives without template copies or authorship, because the
    // editor shows neither. Diff each section against the prior content by
    // ID: re-attach the template copy so editing one section does not throw
    // away the template body of the whole document, and stamp the user only
    // on the sections they actually changed. Carrying the prior stamp on an
    // untouched section is the point — saving the editor re-sends every
    // section, so stamping them all would erase the model's provenance for
    // the whole report on one typo fix.
    for section in &mut content.sections {
        let prior = loaded
            .workspace
            .draft
            .content
            .sections
            .iter()
            .find(|existing| existing.id == section.id);
        if section.template_blocks.is_none() {
            section.template_blocks = prior.and_then(|existing| existing.template_blocks.clone());
        }
        section.authorship = match prior {
            Some(prior)
                if prior.heading == section.heading
                    && prior.blocks == section.blocks
                    && prior.skipped == section.skipped =>
            {
                prior.authorship.clone()
            }
            _ => Some(SectionAuthorship {
                kind: AuthorshipKind::HumanEdited,
                revision: saved_revision,
                model_id: None,
                run_id: None,
                updated_at: now,
            }),
        };
    }

    loaded.workspace.draft = loaded
        .workspace
        .draft
        .replace_content(expected_revision, content, now)
        .map_err(|error| ReportStoreError::InvalidInput(error.to_string()))?;
    mark_template_current(&mut loaded.workspace);
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded.workspace)
}

/// Persist an immutable per-client copy of the redacted template package used
/// by a report. The content hash makes retries idempotent and lets exports keep
/// the original Word layout even if the global template is later deleted.
pub async fn store_report_template_source(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    source_sha256: &str,
    bytes: Vec<u8>,
) -> Result<(), ReportStoreError> {
    if bytes.is_empty() || bytes.len() > 10 * 1024 * 1024 {
        return Err(ReportStoreError::InvalidInput(
            "The writer template source must be between 1 byte and 10 MiB.".to_string(),
        ));
    }
    let actual_sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual_sha256 != source_sha256 {
        return Err(ReportStoreError::InvalidInput(
            "The writer template source did not match its validated content hash.".to_string(),
        ));
    }
    let key = claria_core::s3_keys::report_template_source(client_id, source_sha256);
    match claria_storage::objects::put_object_if_none_match(
        s3,
        bucket,
        &key,
        bytes,
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    )
    .await
    {
        Ok(_) | Err(StorageError::PreconditionFailed { .. }) => Ok(()),
        Err(source) => Err(ReportStoreError::storage(
            "saving the report template formatting",
            source,
        )),
    }
}

/// Replace the accepted draft with content parsed from a managed DOCX. Only
/// structured content, warnings, and a source hash enter the client workspace;
/// the redacted source snapshot is stored as a separate immutable object.
pub async fn apply_report_template(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    expected_revision: u64,
    application: ReportTemplateApplication,
) -> Result<ReportWorkspace, ReportStoreError> {
    let loaded = load_or_create(s3, bucket, client_id).await?;
    apply_report_template_loaded(s3, bucket, loaded, expected_revision, application).await
}

pub async fn apply_report_template_for_report(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    application: ReportTemplateApplication,
) -> Result<ReportWorkspace, ReportStoreError> {
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    apply_report_template_loaded(s3, bucket, loaded, expected_revision, application).await
}

async fn apply_report_template_loaded(
    s3: &S3Client,
    bucket: &str,
    mut loaded: LoadedWorkspace,
    expected_revision: u64,
    application: ReportTemplateApplication,
) -> Result<ReportWorkspace, ReportStoreError> {
    ensure_revision(&loaded.workspace, expected_revision)?;
    ensure_no_active_run(&loaded.workspace)?;
    if loaded.workspace.template_import.is_some() {
        return Err(ReportStoreError::InvalidInput(
            "This Writing session already has a template. Start a new session to use a different template."
                .to_string(),
        ));
    }
    if loaded.workspace.session.pending_proposal.is_some() {
        return Err(ReportStoreError::InvalidInput(
            "Accept or reject the pending proposal before importing a template.".to_string(),
        ));
    }

    let now = jiff::Timestamp::now();
    loaded.workspace.draft = loaded
        .workspace
        .draft
        .replace_content(expected_revision, application.content, now)
        .map_err(|error| ReportStoreError::InvalidInput(error.to_string()))?;
    // Keep an immutable copy of what the template supplied for each section.
    // Skipping a section drops its authored body, and rewriting one replaces
    // it; either way the original stays available for the greyed-out preview
    // and for a later rewrite from the template.
    let template_revision = loaded.workspace.draft.revision;
    for section in &mut loaded.workspace.draft.content.sections {
        section.template_blocks = Some(section.blocks.clone());
        section.authorship = Some(SectionAuthorship {
            kind: AuthorshipKind::Template,
            revision: template_revision,
            model_id: None,
            run_id: None,
            updated_at: now,
        });
    }
    loaded.workspace.template_import = Some(ReportTemplateImport {
        source_sha256: application.source_sha256,
        writer_template_id: Some(application.writer_template_id),
        writer_template_name: Some(application.writer_template_name),
        imported_revision: loaded.workspace.draft.revision,
        imported_at: now,
        warnings: application.warnings,
        reviewed_revision: Some(loaded.workspace.draft.revision),
    });
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded.workspace)
}

pub async fn resolve_report_proposal(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    proposal_id: Uuid,
    decision: ReportProposalDecision,
) -> Result<ReportWorkspace, ReportStoreError> {
    let loaded = load_or_create(s3, bucket, client_id).await?;
    resolve_report_proposal_loaded(s3, bucket, loaded, proposal_id, decision).await
}

pub async fn resolve_report_proposal_for_report(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    proposal_id: Uuid,
    decision: ReportProposalDecision,
) -> Result<ReportWorkspace, ReportStoreError> {
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    resolve_report_proposal_loaded(s3, bucket, loaded, proposal_id, decision).await
}

async fn resolve_report_proposal_loaded(
    s3: &S3Client,
    bucket: &str,
    mut loaded: LoadedWorkspace,
    proposal_id: Uuid,
    decision: ReportProposalDecision,
) -> Result<ReportWorkspace, ReportStoreError> {
    if loaded.workspace.session.pending_proposal.is_none()
        && already_resolved(&loaded.workspace, proposal_id, decision)
    {
        return Ok(loaded.workspace);
    }

    let proposal = loaded
        .workspace
        .session
        .pending_proposal
        .clone()
        .ok_or_else(|| {
            ReportStoreError::InvalidInput(
                "There is no pending report proposal to resolve.".to_string(),
            )
        })?;
    if proposal.id != proposal_id {
        return Err(ReportStoreError::Conflict);
    }

    let now = jiff::Timestamp::now();
    if decision == ReportProposalDecision::Accepted {
        loaded.workspace.draft = loaded
            .workspace
            .draft
            .accept_with_authorship(&proposal, &proposal.model_id, now)
            .map_err(|error| ReportStoreError::InvalidWorkspace(error.to_string()))?;
        // The assistant authored this exact proposal, so accepting it does not
        // create a user-edit queue for the next turn.
        loaded.workspace.session.last_agent_revision = Some(loaded.workspace.draft.revision);
        mark_template_current(&mut loaded.workspace);
    }
    loaded.workspace.session.pending_proposal = None;
    loaded
        .workspace
        .session
        .resolutions
        .push(ReportProposalResolution {
            proposal_id,
            decision,
            resulting_revision: loaded.workspace.draft.revision,
            resolved_at: now,
        });
    if loaded.workspace.session.resolutions.len() > MAX_RESOLUTIONS {
        let remove = loaded.workspace.session.resolutions.len() - MAX_RESOLUTIONS;
        loaded.workspace.session.resolutions.drain(0..remove);
    }
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded.workspace)
}

/// Persist the latest local Word-export outcome as part of the writing
/// session. This status contains no destination path.
pub async fn record_report_export(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    status: ReportExportStatus,
) -> Result<ReportWorkspace, ReportStoreError> {
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    if revision > loaded.workspace.draft.revision {
        return Err(ReportStoreError::Conflict);
    }
    let now = jiff::Timestamp::now();
    loaded.workspace.session.last_export = Some(ReportExport {
        revision,
        status,
        attempted_at: now,
    });
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded.workspace)
}

pub async fn load_export_snapshot(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
) -> Result<ReportExportSnapshot, ReportStoreError> {
    let loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    if loaded.workspace.draft.revision != expected_revision {
        return Err(ReportStoreError::Conflict);
    }
    let mut template_missing = false;
    let template_source = if let Some(template) = &loaded.workspace.template_import {
        let key = claria_core::s3_keys::report_template_source(client_id, &template.source_sha256);
        match claria_storage::objects::get_object_bounded(s3, bucket, &key, 10 * 1024 * 1024).await
        {
            Ok(output) => Some(output.body),
            Err(StorageError::NotFound { .. }) => {
                template_missing = true;
                None
            }
            Err(source) => {
                return Err(ReportStoreError::storage(
                    "loading the report template formatting",
                    source,
                ));
            }
        }
    } else {
        None
    };
    Ok(ReportExportSnapshot {
        draft: loaded.workspace.draft,
        template_source,
        template_missing,
    })
}

/// Soft-delete all report-authoring objects for a client. Versioning retains
/// workspace, attempt, and usage snapshots for lifecycle restoration/audit.
pub async fn delete_report_workspace_for_client(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<usize, ReportStoreError> {
    claria_storage::objects::delete_objects_by_prefix(
        s3,
        bucket,
        &claria_core::s3_keys::report_authoring_client_prefix(client_id),
    )
    .await
    .map_err(|source| ReportStoreError::storage("deleting the report workspace", source))
}

/// Restore every independently stored Writing session (plus its attempt,
/// usage, and template objects) without overwriting concurrent current data.
pub async fn restore_report_workspace_for_client(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<bool, ReportStoreError> {
    ensure_client_exists(s3, bucket, client_id).await?;
    claria_storage::objects::restore_deleted_objects_by_prefix(
        s3,
        bucket,
        &claria_core::s3_keys::report_authoring_client_prefix(client_id),
    )
    .await
    .map_err(|source| ReportStoreError::storage("restoring Writing sessions", source))?;

    // Validate every restored workspace. Returning true when sessions are
    // already current keeps retries and concurrent restores idempotent.
    Ok(!list_report_workspaces(s3, bucket, client_id)
        .await?
        .is_empty())
}

pub fn suggested_docx_filename(title: &str) -> String {
    let mut value = String::new();
    let mut previous_dash = false;
    for character in title.trim().chars() {
        let safe = if character.is_alphanumeric() {
            Some(character)
        } else if character.is_whitespace() || matches!(character, '-' | '_') {
            Some('-')
        } else {
            None
        };
        if let Some(character) = safe {
            if character == '-' {
                if previous_dash || value.is_empty() {
                    continue;
                }
                previous_dash = true;
            } else {
                previous_dash = false;
            }
            value.push(character);
        }
        if value.chars().count() >= 80 {
            break;
        }
    }
    while value.ends_with('-') {
        value.pop();
    }
    if value.is_empty() {
        value.push_str("report");
    }
    format!("{value}.docx")
}

/// A workspace object read together with the concurrency token that lets it
/// be written back. Callers that run multi-step work against one workspace
/// (the writer turn loop) hold this across the whole operation and save it
/// once at the end.
pub struct LoadedWorkspace {
    pub workspace: ReportWorkspace,
    /// The S3 key this workspace was read from — session or legacy singleton.
    pub key: String,
    /// ETag of the body currently stored at `key`; every save is conditional
    /// on it and refreshes it in place.
    pub etag: String,
}

/// Load this client's legacy singleton workspace, creating an empty one if it
/// does not exist yet.
pub async fn load_or_create(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<LoadedWorkspace, ReportStoreError> {
    if client_id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "Client ID must not be nil.".to_string(),
        ));
    }
    ensure_client_exists(s3, bucket, client_id).await?;
    let key = claria_core::s3_keys::report_workspace(client_id);
    match load_existing(s3, bucket, &key, client_id).await {
        Ok(loaded) => Ok(loaded),
        Err(LoadWorkspaceError::NotFound) => {
            let workspace = ReportWorkspace::new(client_id, jiff::Timestamp::now());
            workspace
                .validate()
                .map_err(|error| ReportStoreError::InvalidWorkspace(error.to_string()))?;
            match claria_storage::state::save_state_if_none_match(s3, bucket, &key, &workspace)
                .await
            {
                Ok(etag) => Ok(LoadedWorkspace {
                    workspace,
                    key,
                    etag: require_etag(etag)?,
                }),
                Err(StorageError::PreconditionFailed { .. }) => {
                    load_existing(s3, bucket, &key, client_id)
                        .await
                        .map_err(LoadWorkspaceError::into_public)
                }
                Err(source) => Err(ReportStoreError::storage(
                    "creating the report workspace",
                    source,
                )),
            }
        }
        Err(error) => Err(error.into_public()),
    }
}

/// Load one independently resumable Writing session by ID, falling back to
/// the pre-multi-session singleton when it holds that report.
pub async fn load_for_report(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<LoadedWorkspace, ReportStoreError> {
    let session_key = claria_core::s3_keys::report_session_workspace(client_id, report_id);
    match load_existing(s3, bucket, &session_key, client_id).await {
        Ok(loaded) if loaded.workspace.report_id == report_id => return Ok(loaded),
        Ok(_) => {
            return Err(ReportStoreError::InvalidWorkspace(
                "The Writing session ID does not match its storage key.".to_string(),
            ));
        }
        Err(LoadWorkspaceError::NotFound) => {}
        Err(error) => return Err(error.into_public()),
    }

    // Backward compatibility for the pre-multi-session singleton.
    let legacy_key = claria_core::s3_keys::report_workspace(client_id);
    match load_existing(s3, bucket, &legacy_key, client_id).await {
        Ok(loaded) if loaded.workspace.report_id == report_id => Ok(loaded),
        Ok(_) | Err(LoadWorkspaceError::NotFound) => Err(ReportStoreError::InvalidInput(
            "That Writing session is no longer available.".to_string(),
        )),
        Err(error) => Err(error.into_public()),
    }
}

async fn ensure_client_exists(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<(), ReportStoreError> {
    let key = claria_core::s3_keys::client(client_id);
    let output = claria_storage::objects::get_object(s3, bucket, &key)
        .await
        .map_err(|source| match source {
            StorageError::NotFound { .. } => ReportStoreError::ClientNotFound,
            other => ReportStoreError::storage("validating the client", other),
        })?;
    let client: Client = serde_json::from_slice(&output.body).map_err(|_| {
        ReportStoreError::InvalidWorkspace("the client record is invalid".to_string())
    })?;
    if client.id != client_id || client_id.is_nil() {
        return Err(ReportStoreError::InvalidWorkspace(
            "the client record ID does not match its key".to_string(),
        ));
    }
    Ok(())
}

enum LoadWorkspaceError {
    NotFound,
    Public(ReportStoreError),
}

impl LoadWorkspaceError {
    fn into_public(self) -> ReportStoreError {
        match self {
            Self::NotFound => {
                ReportStoreError::InvalidWorkspace("Report workspace was not found.".to_string())
            }
            Self::Public(error) => error,
        }
    }
}

async fn load_existing(
    s3: &S3Client,
    bucket: &str,
    key: &str,
    client_id: Uuid,
) -> Result<LoadedWorkspace, LoadWorkspaceError> {
    let output = claria_storage::objects::get_object(s3, bucket, key)
        .await
        .map_err(|source| match source {
            StorageError::NotFound { .. } => LoadWorkspaceError::NotFound,
            other => LoadWorkspaceError::Public(ReportStoreError::storage(
                "loading the report workspace",
                other,
            )),
        })?;
    let workspace = decode_report_workspace(&output.body).map_err(|error| {
        LoadWorkspaceError::Public(ReportStoreError::InvalidWorkspace(error.to_string()))
    })?;
    if workspace.client_id != client_id {
        return Err(LoadWorkspaceError::Public(
            ReportStoreError::InvalidWorkspace(
                "The report workspace belongs to another client.".to_string(),
            ),
        ));
    }
    let etag = require_etag(output.etag.unwrap_or_default()).map_err(LoadWorkspaceError::Public)?;
    Ok(LoadedWorkspace {
        workspace,
        key: key.to_string(),
        etag,
    })
}

/// Validate and conditionally write a loaded workspace back to its key,
/// refreshing its ETag. A concurrent write elsewhere surfaces as
/// [`ReportStoreError::Conflict`] and leaves the stored object untouched.
pub async fn save_loaded(
    s3: &S3Client,
    bucket: &str,
    loaded: &mut LoadedWorkspace,
) -> Result<(), ReportStoreError> {
    loaded
        .workspace
        .validate()
        .map_err(|error| ReportStoreError::InvalidWorkspace(error.to_string()))?;
    let etag = claria_storage::state::save_state_if_match(
        s3,
        bucket,
        &loaded.key,
        &loaded.workspace,
        &loaded.etag,
    )
    .await
    .map_err(|source| match source {
        StorageError::PreconditionFailed { .. } => ReportStoreError::Conflict,
        other => ReportStoreError::storage("saving the report workspace", other),
    })?;
    loaded.etag = require_etag(etag)?;
    Ok(())
}

fn require_etag(etag: String) -> Result<String, ReportStoreError> {
    if etag.trim().is_empty() {
        Err(ReportStoreError::InvalidWorkspace(
            "S3 did not return an ETag for the report workspace.".to_string(),
        ))
    } else {
        Ok(etag)
    }
}

/// Refuse to mutate a report a whole-report drafting run currently owns.
///
/// The pointer is set for the life of one run and cleared when it finishes or
/// fails, so in practice this only fires on a second window racing the first.
/// Landing a section and cutting a revision underneath each other is exactly
/// the collision it exists to stop.
pub fn ensure_no_active_run(workspace: &ReportWorkspace) -> Result<(), ReportStoreError> {
    if workspace.active_run_id.is_some() {
        Err(ReportStoreError::InvalidInput(
            ACTIVE_RUN_MESSAGE.to_string(),
        ))
    } else {
        Ok(())
    }
}

/// Refuse to act on a workspace the caller has a stale view of.
pub fn ensure_revision(
    workspace: &ReportWorkspace,
    expected_revision: u64,
) -> Result<(), ReportStoreError> {
    if workspace.draft.revision != expected_revision {
        Err(ReportStoreError::Conflict)
    } else {
        Ok(())
    }
}

/// Record that the imported template has been reviewed at the draft's current
/// revision, so an edit does not leave a stale "template changed" banner.
pub fn mark_template_current(workspace: &mut ReportWorkspace) {
    if let Some(template) = &mut workspace.template_import {
        template.reviewed_revision = Some(workspace.draft.revision);
    }
}

fn already_resolved(
    workspace: &ReportWorkspace,
    proposal_id: Uuid,
    decision: ReportProposalDecision,
) -> bool {
    workspace
        .session
        .resolutions
        .iter()
        .any(|resolution| resolution.proposal_id == proposal_id && resolution.decision == decision)
}
