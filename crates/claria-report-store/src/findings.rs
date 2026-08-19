//! Durable review findings: the object protocol, the sweep write-back, and
//! the three user actions that resolve one finding.
//!
//! Nothing here talks to Bedrock. A review sweep hands this module finished
//! [`Finding`] rows; applying and undoing a style replacement is pure text
//! work against the accepted draft.

use aws_sdk_s3::Client as S3Client;
use claria_core::models::{
    findings::{
        Finding, FindingAction, FindingStatus, MAX_REPORT_FINDINGS, ReportFindings, ReviewCoverage,
        ReviewPass, decode_report_findings, finding_is_stale,
    },
    report::{AuthorshipKind, ReportBlock, ReportContent, ReportWorkspace, SectionAuthorship},
};
use claria_storage::error::StorageError;
use jiff::Timestamp;
use uuid::Uuid;

use crate::{
    ReportStoreError,
    workspace::{LoadedWorkspace, load_for_report, mark_template_current, save_loaded},
};

/// A findings object read together with the concurrency token that lets it be
/// written back.
///
/// Unlike a workspace, a findings object may legitimately not exist yet: a
/// report nobody has reviewed has no findings. `etag: None` means exactly
/// that, and the first save is conditional on the object still being absent.
pub struct LoadedFindings {
    pub findings: ReportFindings,
    pub key: String,
    pub etag: Option<String>,
}

/// What a resolved finding leaves behind: the report as it now stands and the
/// findings that describe it.
///
/// Dismissing a finding does not touch the report, but it still returns the
/// workspace so the caller has one shape to render for every action.
#[derive(Debug, Clone)]
pub struct FindingOutcome {
    pub workspace: ReportWorkspace,
    pub findings: ReportFindings,
}

/// Load this report's findings, treating a missing object as an empty set.
pub async fn load_report_findings(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<LoadedFindings, ReportStoreError> {
    if client_id.is_nil() || report_id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "Client and report IDs must not be nil.".to_string(),
        ));
    }
    let key = claria_core::s3_keys::report_findings(client_id, report_id);
    match claria_storage::objects::get_object(s3, bucket, &key).await {
        Ok(output) => {
            let findings = decode_report_findings(&output.body)
                .map_err(|error| ReportStoreError::InvalidFindings(error.to_string()))?;
            if findings.client_id != client_id || findings.report_id != report_id {
                return Err(ReportStoreError::InvalidFindings(
                    "The stored review findings belong to another report.".to_string(),
                ));
            }
            let etag = require_etag(output.etag.unwrap_or_default())?;
            Ok(LoadedFindings {
                findings,
                key,
                etag: Some(etag),
            })
        }
        Err(StorageError::NotFound { .. }) => Ok(LoadedFindings {
            findings: ReportFindings::new(client_id, report_id, jiff::Timestamp::now()),
            key,
            etag: None,
        }),
        Err(source) => Err(ReportStoreError::storage("loading review findings", source)),
    }
}

/// Validate and conditionally write findings back, refreshing the ETag.
///
/// A never-saved object is written with `If-None-Match`, so two computers
/// producing the first review at once cannot silently clobber one another; an
/// existing one is written with `If-Match`. Either precondition failing is a
/// [`ReportStoreError::Conflict`].
pub async fn save_report_findings(
    s3: &S3Client,
    bucket: &str,
    loaded: &mut LoadedFindings,
) -> Result<(), ReportStoreError> {
    loaded
        .findings
        .validate()
        .map_err(|error| ReportStoreError::InvalidFindings(error.to_string()))?;
    let saved = match &loaded.etag {
        None => {
            claria_storage::state::save_state_if_none_match(
                s3,
                bucket,
                &loaded.key,
                &loaded.findings,
            )
            .await
        }
        Some(etag) => {
            claria_storage::state::save_state_if_match(
                s3,
                bucket,
                &loaded.key,
                &loaded.findings,
                etag,
            )
            .await
        }
    };
    let etag = saved.map_err(|source| match source {
        StorageError::PreconditionFailed { .. } => ReportStoreError::Conflict,
        other => ReportStoreError::storage("saving review findings", other),
    })?;
    loaded.etag = Some(require_etag(etag)?);
    Ok(())
}

/// Fold one review sweep's results into the stored findings.
///
/// Prior open and invalidated rows are dropped — the sweep just re-derived
/// them against the current revision — while applied and dismissed rows stay
/// as the user's decision history. If the total would exceed
/// [`MAX_REPORT_FINDINGS`], the oldest resolved rows are dropped first, so a
/// fresh finding never loses its place to a year-old dismissal.
pub fn replace_findings_for_revision(
    findings: &mut ReportFindings,
    revision: u64,
    fresh: Vec<Finding>,
    coverage: Vec<ReviewCoverage>,
    now: Timestamp,
) -> Result<(), ReportStoreError> {
    let mut fresh = fresh;
    for finding in &fresh {
        if finding.anchor.revision != revision {
            return Err(ReportStoreError::InvalidInput(
                "A review finding is anchored to a revision the sweep did not read.".to_string(),
            ));
        }
        if finding.status != FindingStatus::Open {
            return Err(ReportStoreError::InvalidInput(
                "A newly reviewed finding must start out open.".to_string(),
            ));
        }
    }

    let mut history: Vec<Finding> = findings
        .findings
        .drain(..)
        .filter(|finding| {
            matches!(
                finding.status,
                FindingStatus::Applied | FindingStatus::Dismissed
            )
        })
        .collect();
    history.sort_by_key(|finding| finding.created_at);

    fresh.truncate(MAX_REPORT_FINDINGS);
    let room = MAX_REPORT_FINDINGS - fresh.len();
    if history.len() > room {
        history.drain(0..history.len() - room);
    }
    history.extend(fresh);

    findings.findings = history;
    findings.coverage = coverage;
    findings.updated_at = now;
    findings
        .validate()
        .map_err(|error| ReportStoreError::InvalidFindings(error.to_string()))
}

/// Stamp every open finding whose anchor has gone stale as
/// [`FindingStatus::Invalidated`], returning how many changed.
///
/// This is a display cache, nothing more. `finding_is_stale` is evaluated on
/// every read and is the only authority; a caller that never runs this still
/// behaves correctly, which is why the list path writes the result back
/// best-effort rather than failing on it.
pub fn refresh_finding_statuses(
    findings: &mut ReportFindings,
    content: &ReportContent,
    now: Timestamp,
) -> usize {
    let mut changed = 0;
    for finding in &mut findings.findings {
        if finding.status == FindingStatus::Open && finding_is_stale(finding, content) {
            finding.status = FindingStatus::Invalidated;
            finding.resolved_at = Some(now);
            changed += 1;
        }
    }
    if changed > 0 {
        findings.updated_at = now;
    }
    changed
}

/// Every finding recorded against one report, with stale open findings
/// refreshed.
pub async fn list_report_findings(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<ReportFindings, ReportStoreError> {
    let workspace = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut loaded = load_report_findings(s3, bucket, client_id, report_id).await?;
    let refreshed = refresh_finding_statuses(
        &mut loaded.findings,
        &workspace.workspace.draft.content,
        jiff::Timestamp::now(),
    );
    if refreshed > 0 {
        // Deliberately not propagated: the statuses returned below are already
        // right, and a failed cache write must not stop the user seeing their
        // findings. A later read re-derives the same answer.
        let _ = save_report_findings(s3, bucket, &mut loaded).await;
    }
    Ok(loaded.findings)
}

/// Carry out one user decision about one finding.
///
/// The three actions differ enough to be worth reading separately, so each
/// keeps its own entry point; this is the single dispatch the command layer
/// calls so the wire action never has to be re-matched there.
pub async fn resolve_finding(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    finding_id: Uuid,
    action: FindingAction,
) -> Result<FindingOutcome, ReportStoreError> {
    match action {
        FindingAction::ApplyStyle => {
            apply_style_finding(s3, bucket, client_id, report_id, finding_id).await
        }
        FindingAction::UndoStyle => {
            undo_style_finding(s3, bucket, client_id, report_id, finding_id).await
        }
        FindingAction::Dismiss => {
            dismiss_finding(s3, bucket, client_id, report_id, finding_id).await
        }
    }
}

/// Apply a style finding's replacement to the accepted draft.
///
/// The finding must be open, must belong to the style pass, and its anchor
/// must still be fresh. The replacement text must occur exactly once in the
/// target paragraph: zero matches means the passage was rewritten underneath
/// the review, and more than one means the anchor cannot say which occurrence
/// was meant. Both refuse the edit and mark the finding invalidated, because
/// neither can become applicable again without a new review.
///
/// The workspace is saved before the findings object. If the findings save
/// then fails the caller gets an error, but the edit itself stands as a real
/// revision — the report is the durable artifact and the finding's status is
/// bookkeeping about it. The other order would risk a finding marked applied
/// against a revision that was never cut.
pub async fn apply_style_finding(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    finding_id: Uuid,
) -> Result<FindingOutcome, ReportStoreError> {
    let mut workspace = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut loaded = load_report_findings(s3, bucket, client_id, report_id).await?;
    let now = jiff::Timestamp::now();

    let finding = require_finding(&loaded, finding_id)?;
    if finding.status != FindingStatus::Open {
        return Err(ReportStoreError::InvalidInput(
            "Only an open review finding can be applied.".to_string(),
        ));
    }
    if finding.pass != ReviewPass::Style {
        return Err(ReportStoreError::InvalidInput(
            "Consistency findings are read-only; there is nothing to apply.".to_string(),
        ));
    }
    let Some(proposal) = finding.proposal.clone() else {
        return Err(ReportStoreError::InvalidInput(
            "That review finding does not propose a replacement.".to_string(),
        ));
    };
    ensure_no_pending_proposal(&workspace)?;

    if finding_is_stale(&finding, &workspace.workspace.draft.content) {
        return Err(invalidate(
            s3,
            bucket,
            &mut loaded,
            finding_id,
            "the section has been rewritten since the review ran",
            now,
        )
        .await);
    }

    let Some(section_index) = section_index(&workspace, finding.anchor.section_id) else {
        return Err(invalidate(
            s3,
            bucket,
            &mut loaded,
            finding_id,
            "the section it points at is gone",
            now,
        )
        .await);
    };
    let block_index = usize::try_from(proposal.block_index).unwrap_or(usize::MAX);
    let Some(text) = paragraph_text(&workspace, section_index, block_index) else {
        return Err(invalidate(
            s3,
            bucket,
            &mut loaded,
            finding_id,
            "the paragraph it points at is no longer there",
            now,
        )
        .await);
    };
    match text.matches(&proposal.original_text).count() {
        1 => {}
        0 => {
            return Err(invalidate(
                s3,
                bucket,
                &mut loaded,
                finding_id,
                "the text it replaces has changed",
                now,
            )
            .await);
        }
        _ => {
            return Err(invalidate(
                s3,
                bucket,
                &mut loaded,
                finding_id,
                "the text it replaces now appears more than once, so the anchor is ambiguous",
                now,
            )
            .await);
        }
    }
    let replaced = text.replacen(&proposal.original_text, &proposal.replacement_text, 1);

    // The model wrote the replacement, so the section is model-revised and
    // credited to the reviewer that proposed it.
    let revision = cut_revision(
        &mut workspace,
        section_index,
        block_index,
        replaced,
        AuthorshipKind::ModelRevised,
        Some(finding.model_id.clone()),
        now,
    )?;
    save_loaded(s3, bucket, &mut workspace).await?;

    if let Some(finding) = loaded.findings.find_mut(finding_id) {
        finding.status = FindingStatus::Applied;
        finding.applied_revision = Some(revision);
        finding.resolved_at = Some(now);
    }
    loaded.findings.updated_at = now;
    save_report_findings(s3, bucket, &mut loaded).await?;

    Ok(FindingOutcome {
        workspace: workspace.workspace,
        findings: loaded.findings,
    })
}

/// Undo an applied style replacement as a new revision.
///
/// The inverse replacement is checked against the paragraph as it stands now,
/// not against the revision the finding was applied to, and must match exactly
/// once for the same reasons the forward direction must. Anchor freshness is
/// deliberately *not* required: applying the finding stamped the section
/// itself, and any later edit would too, so a freshness gate would make an
/// applied finding un-undoable the moment anything else in the section moved.
///
/// The restored section is stamped [`AuthorshipKind::HumanEdited`]: whatever
/// the model proposed, the text that now stands is there because the user
/// chose to put it back. The reopened finding is re-anchored to the revision
/// the undo cut, so it is immediately actionable again rather than derived
/// stale by the stamp the undo itself wrote.
pub async fn undo_style_finding(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    finding_id: Uuid,
) -> Result<FindingOutcome, ReportStoreError> {
    let mut workspace = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut loaded = load_report_findings(s3, bucket, client_id, report_id).await?;
    let now = jiff::Timestamp::now();

    let finding = require_finding(&loaded, finding_id)?;
    if finding.status != FindingStatus::Applied {
        return Err(ReportStoreError::InvalidInput(
            "Only an applied review finding can be undone.".to_string(),
        ));
    }
    let Some(proposal) = finding.proposal.clone() else {
        return Err(ReportStoreError::InvalidInput(
            "That review finding does not propose a replacement.".to_string(),
        ));
    };
    ensure_no_pending_proposal(&workspace)?;

    let section_index = section_index(&workspace, finding.anchor.section_id)
        .ok_or_else(|| unresolvable_undo("the section it changed is gone"))?;
    let block_index = usize::try_from(proposal.block_index).unwrap_or(usize::MAX);
    let text = paragraph_text(&workspace, section_index, block_index)
        .ok_or_else(|| unresolvable_undo("the paragraph it changed is no longer there"))?;
    match text.matches(&proposal.replacement_text).count() {
        1 => {}
        0 => return Err(unresolvable_undo("the replacement text has since changed")),
        _ => {
            return Err(unresolvable_undo(
                "the replacement text now appears more than once, so the anchor is ambiguous",
            ));
        }
    }
    let restored = text.replacen(&proposal.replacement_text, &proposal.original_text, 1);

    let revision = cut_revision(
        &mut workspace,
        section_index,
        block_index,
        restored,
        AuthorshipKind::HumanEdited,
        None,
        now,
    )?;
    save_loaded(s3, bucket, &mut workspace).await?;

    if let Some(finding) = loaded.findings.find_mut(finding_id) {
        finding.status = FindingStatus::Open;
        finding.applied_revision = None;
        finding.resolved_at = None;
        // Re-anchor to the revision the undo cut. The undo just proved, byte
        // for byte, that the reviewed passage is back, and undo stamps the
        // section — without this the finding would reopen and be derived stale
        // in the same breath, which is not a state the user could act on.
        finding.anchor.revision = revision;
    }
    loaded.findings.updated_at = now;
    save_report_findings(s3, bucket, &mut loaded).await?;

    Ok(FindingOutcome {
        workspace: workspace.workspace,
        findings: loaded.findings,
    })
}

/// Set an open finding aside without changing the report.
pub async fn dismiss_finding(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    finding_id: Uuid,
) -> Result<FindingOutcome, ReportStoreError> {
    let workspace = load_for_report(s3, bucket, client_id, report_id).await?;
    let mut loaded = load_report_findings(s3, bucket, client_id, report_id).await?;
    let now = jiff::Timestamp::now();

    let finding = require_finding(&loaded, finding_id)?;
    if finding.status != FindingStatus::Open {
        return Err(ReportStoreError::InvalidInput(
            "Only an open review finding can be dismissed.".to_string(),
        ));
    }
    if let Some(finding) = loaded.findings.find_mut(finding_id) {
        finding.status = FindingStatus::Dismissed;
        finding.resolved_at = Some(now);
    }
    loaded.findings.updated_at = now;
    save_report_findings(s3, bucket, &mut loaded).await?;

    Ok(FindingOutcome {
        workspace: workspace.workspace,
        findings: loaded.findings,
    })
}

/// Replace one paragraph, cut a new revision, and stamp the section that
/// changed. Returns the revision the cut produced.
fn cut_revision(
    workspace: &mut LoadedWorkspace,
    section_index: usize,
    block_index: usize,
    text: String,
    kind: AuthorshipKind,
    model_id: Option<String>,
    now: Timestamp,
) -> Result<u64, ReportStoreError> {
    let mut content = workspace.workspace.draft.content.clone();
    content.sections[section_index].blocks[block_index] = ReportBlock::Paragraph { text };
    let expected_revision = workspace.workspace.draft.revision;
    let mut draft = workspace
        .workspace
        .draft
        .replace_content(expected_revision, content, now)
        .map_err(|error| ReportStoreError::InvalidInput(error.to_string()))?;
    let revision = draft.revision;
    draft.content.sections[section_index].authorship = Some(SectionAuthorship {
        kind,
        revision,
        model_id,
        run_id: None,
        updated_at: now,
    });
    workspace.workspace.draft = draft;
    mark_template_current(&mut workspace.workspace);
    workspace.workspace.updated_at = now;
    Ok(revision)
}

fn require_finding(loaded: &LoadedFindings, finding_id: Uuid) -> Result<Finding, ReportStoreError> {
    loaded.findings.find(finding_id).cloned().ok_or_else(|| {
        ReportStoreError::InvalidInput("That review finding no longer exists.".to_string())
    })
}

fn ensure_no_pending_proposal(workspace: &LoadedWorkspace) -> Result<(), ReportStoreError> {
    if workspace.workspace.session.pending_proposal.is_some() {
        return Err(ReportStoreError::InvalidInput(
            "Accept or reject the pending proposal before resolving review findings.".to_string(),
        ));
    }
    Ok(())
}

fn section_index(workspace: &LoadedWorkspace, section_id: Uuid) -> Option<usize> {
    workspace
        .workspace
        .draft
        .content
        .sections
        .iter()
        .position(|section| section.id == section_id)
}

fn paragraph_text(
    workspace: &LoadedWorkspace,
    section_index: usize,
    block_index: usize,
) -> Option<String> {
    match workspace.workspace.draft.content.sections[section_index]
        .blocks
        .get(block_index)?
    {
        // Only paragraphs carry an anchored replacement in v1. A finding
        // pointing at a list or a table can never be applied, so it is treated
        // exactly like one whose paragraph has gone.
        ReportBlock::Paragraph { text } => Some(text.clone()),
        ReportBlock::BulletList { .. } | ReportBlock::Table { .. } => None,
    }
}

/// Mark a finding invalidated, persist that, and produce the error explaining
/// why the user's click did nothing.
///
/// A failure to persist the invalidation is returned instead: it is the more
/// actionable of the two, and the finding stays open so a retry sees the same
/// refusal.
async fn invalidate(
    s3: &S3Client,
    bucket: &str,
    loaded: &mut LoadedFindings,
    finding_id: Uuid,
    reason: &str,
    now: Timestamp,
) -> ReportStoreError {
    if let Some(finding) = loaded.findings.find_mut(finding_id) {
        finding.status = FindingStatus::Invalidated;
        finding.resolved_at = Some(now);
    }
    loaded.findings.updated_at = now;
    if let Err(error) = save_report_findings(s3, bucket, loaded).await {
        return error;
    }
    ReportStoreError::InvalidInput(format!(
        "This finding no longer applies: {reason}. Run the review again to refresh it."
    ))
}

fn unresolvable_undo(reason: &str) -> ReportStoreError {
    ReportStoreError::InvalidInput(format!(
        "This change cannot be undone automatically: {reason}. Restore an earlier revision instead."
    ))
}

fn require_etag(etag: String) -> Result<String, ReportStoreError> {
    if etag.trim().is_empty() {
        Err(ReportStoreError::InvalidFindings(
            "S3 did not return an ETag for the review findings.".to_string(),
        ))
    } else {
        Ok(etag)
    }
}
