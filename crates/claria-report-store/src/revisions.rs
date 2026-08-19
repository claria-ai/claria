//! Immutable workspace revisions: S3 object-version summaries, revision
//! restore, and the discard-queued-edits baseline restore.

use std::collections::HashSet;

use aws_sdk_s3::Client as S3Client;
use claria_core::models::report::{ReportDraft, ReportWorkspace, decode_report_workspace};
use uuid::Uuid;

use crate::{
    ReportRevisionSummary, ReportStoreError,
    workspace::{
        ensure_no_active_run, ensure_revision, load_for_report, mark_template_current, save_loaded,
    },
};

pub async fn list_report_revisions(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    cache: &RevisionCache,
) -> Result<Vec<ReportRevisionSummary>, ReportStoreError> {
    let current = load_for_report(s3, bucket, client_id, report_id).await?;

    let mut seen = HashSet::new();
    let mut revisions = Vec::new();
    for (_, summary) in load_revision_summaries(s3, bucket, client_id, &current.key, cache).await? {
        if summary.report_id == report_id && seen.insert(summary.revision) {
            revisions.push(ReportRevisionSummary {
                revision: summary.revision,
                title: summary.title,
                updated_at: summary.updated_at,
            });
        }
    }
    revisions.sort_by_key(|revision| std::cmp::Reverse(revision.revision));
    Ok(revisions)
}

pub async fn load_report_revision(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    cache: &RevisionCache,
) -> Result<ReportDraft, ReportStoreError> {
    let current = load_for_report(s3, bucket, client_id, report_id).await?;
    Ok(load_workspace_revision(
        s3,
        bucket,
        client_id,
        report_id,
        revision,
        &current.key,
        cache,
    )
    .await?
    .draft)
}

pub async fn revert_report_revision(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    revision: u64,
    cache: &RevisionCache,
) -> Result<ReportWorkspace, ReportStoreError> {
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    ensure_revision(&loaded.workspace, expected_revision)?;
    ensure_no_active_run(&loaded.workspace)?;
    if revision >= expected_revision {
        return Err(ReportStoreError::InvalidInput(
            "Choose an earlier report revision to restore.".to_string(),
        ));
    }
    if loaded.workspace.session.pending_proposal.is_some() {
        return Err(ReportStoreError::InvalidInput(
            "Accept or reject the pending proposal before restoring a report revision.".to_string(),
        ));
    }
    let historical = load_workspace_revision(
        s3,
        bucket,
        client_id,
        report_id,
        revision,
        &loaded.key,
        cache,
    )
    .await?;

    let now = jiff::Timestamp::now();
    loaded.workspace.draft = loaded
        .workspace
        .draft
        .replace_content(expected_revision, historical.draft.content, now)
        .map_err(|error| ReportStoreError::InvalidInput(error.to_string()))?;
    // A revert restores report content, not session-level provenance. Keep the
    // current proposal/template metadata and save the historical content as a
    // new draft revision.
    mark_template_current(&mut loaded.workspace);
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded.workspace)
}

/// Discard report content saved since the assistant's last completed turn.
/// The prior content is restored as a new immutable revision and marked as
/// the current baseline, so it is no longer queued into the next message.
pub async fn discard_queued_report_edits(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    cache: &RevisionCache,
) -> Result<ReportWorkspace, ReportStoreError> {
    let mut loaded = load_for_report(s3, bucket, client_id, report_id).await?;
    ensure_revision(&loaded.workspace, expected_revision)?;
    ensure_no_active_run(&loaded.workspace)?;
    if loaded.workspace.session.pending_proposal.is_some() {
        return Err(ReportStoreError::InvalidInput(
            "Accept or reject the pending proposal before discarding report edits.".to_string(),
        ));
    }
    let baseline_revision = loaded.workspace.session.last_agent_revision.unwrap_or(0);
    if baseline_revision >= expected_revision {
        return Err(ReportStoreError::InvalidInput(
            "There are no saved report edits queued for the next message.".to_string(),
        ));
    }
    let historical = load_workspace_revision(
        s3,
        bucket,
        client_id,
        report_id,
        baseline_revision,
        &loaded.key,
        cache,
    )
    .await?;

    let now = jiff::Timestamp::now();
    loaded.workspace.draft = loaded
        .workspace
        .draft
        .replace_content(expected_revision, historical.draft.content, now)
        .map_err(|error| ReportStoreError::InvalidInput(error.to_string()))?;
    loaded.workspace.template_import = historical.template_import;
    loaded.workspace.session.last_agent_revision = Some(loaded.workspace.draft.revision);
    loaded.workspace.updated_at = now;
    save_loaded(s3, bucket, &mut loaded).await?;
    Ok(loaded.workspace)
}

/// What the revision surfaces need to know about one immutable stored
/// version of the workspace object.
#[derive(Debug, Clone)]
struct RevisionSummary {
    report_id: Uuid,
    revision: u64,
    title: String,
    updated_at: jiff::Timestamp,
}

/// Bounded number of per-version summaries held in memory.
const REVISION_CACHE_CAPACITY: usize = 1024;

/// LRU cache of workspace-version summaries keyed by S3 version ID.
///
/// A version ID names an immutable body, so a cached summary can never go
/// stale — no ETag revalidation is needed, unlike the record cache. This is
/// what turns the revision list from N full-body GETs into GETs for only the
/// versions this process has never seen.
pub struct RevisionCache {
    inner: std::sync::Mutex<lru::LruCache<String, RevisionSummary>>,
}

impl Default for RevisionCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RevisionCache {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(REVISION_CACHE_CAPACITY).expect("nonzero"),
            )),
        }
    }

    fn get(&self, workspace_key: &str, version_id: &str) -> Option<RevisionSummary> {
        self.inner
            .lock()
            .expect("revision cache lock poisoned")
            .get(&format!("{workspace_key}\0{version_id}"))
            .cloned()
    }

    fn insert(&self, workspace_key: &str, version_id: String, summary: RevisionSummary) {
        self.inner
            .lock()
            .expect("revision cache lock poisoned")
            .put(format!("{workspace_key}\0{version_id}"), summary);
    }
}

/// Summaries for every non-delete-marker version of the workspace object, in
/// listing order (newest first). Uncached versions are fetched with bounded
/// concurrency; cached ones cost no GET.
async fn load_revision_summaries(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    workspace_key: &str,
    cache: &RevisionCache,
) -> Result<Vec<(String, RevisionSummary)>, ReportStoreError> {
    use futures::stream::StreamExt;

    let versions = claria_storage::objects::list_object_versions(s3, bucket, workspace_key)
        .await
        .map_err(|source| ReportStoreError::storage("listing report revisions", source))?;

    let lookups = versions
        .into_iter()
        .filter(|version| !version.is_delete_marker)
        .map(|version| async move {
            if let Some(summary) = cache.get(workspace_key, &version.version_id) {
                return Ok((version.version_id, summary));
            }
            let workspace = load_workspace_object_version(
                s3,
                bucket,
                workspace_key,
                client_id,
                &version.version_id,
            )
            .await?;
            let summary = RevisionSummary {
                report_id: workspace.report_id,
                revision: workspace.draft.revision,
                title: workspace.draft.content.title,
                updated_at: workspace.draft.updated_at,
            };
            cache.insert(workspace_key, version.version_id.clone(), summary.clone());
            Ok::<_, ReportStoreError>((version.version_id, summary))
        })
        .collect::<Vec<_>>();

    futures::stream::iter(lookups)
        .buffered(claria_storage::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect()
}

async fn load_workspace_revision(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    workspace_key: &str,
    cache: &RevisionCache,
) -> Result<ReportWorkspace, ReportStoreError> {
    // The summaries name which immutable version holds the wanted revision,
    // so only that one body is fetched.
    let summaries = load_revision_summaries(s3, bucket, client_id, workspace_key, cache).await?;
    for (version_id, summary) in summaries {
        if summary.report_id == report_id && summary.revision == revision {
            return load_workspace_object_version(
                s3,
                bucket,
                workspace_key,
                client_id,
                &version_id,
            )
            .await;
        }
    }
    Err(ReportStoreError::InvalidInput(format!(
        "Report revision {revision} is no longer available."
    )))
}

async fn load_workspace_object_version(
    s3: &S3Client,
    bucket: &str,
    key: &str,
    client_id: Uuid,
    version_id: &str,
) -> Result<ReportWorkspace, ReportStoreError> {
    let output = claria_storage::objects::get_object_version(s3, bucket, key, version_id)
        .await
        .map_err(|source| ReportStoreError::storage("reading a report revision", source))?;
    let workspace = decode_report_workspace(&output.body).map_err(|error| {
        ReportStoreError::InvalidWorkspace(format!("a stored report revision is invalid: {error}"))
    })?;
    if workspace.client_id != client_id {
        return Err(ReportStoreError::InvalidWorkspace(
            "A report revision belongs to another client.".to_string(),
        ));
    }
    Ok(workspace)
}
