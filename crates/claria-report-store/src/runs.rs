//! Drafting-run persistence: create-once objects, ETag-conditional saves, and
//! the per-session listing.
//!
//! A run is rewritten after every section the writer lands, so every save here
//! is conditional: a run that was resumed on another computer conflicts rather
//! than losing landed sections.

use aws_sdk_s3::Client as S3Client;
use claria_core::models::report_run::{DraftRun, decode_draft_run};
use claria_storage::error::StorageError;
use uuid::Uuid;

use crate::ReportStoreError;

/// A drafting run read together with the concurrency token that lets it be
/// written back. The run executor holds one of these for the life of a run and
/// saves it after every landed section.
#[derive(Debug, Clone)]
pub struct LoadedRun {
    pub run: DraftRun,
    pub key: String,
    /// ETag of the body currently stored at `key`; every save is conditional
    /// on it and refreshes it in place.
    pub etag: String,
}

/// Write a new drafting run, refusing to overwrite an existing one. A retried
/// create for a run ID that already exists is a conflict, not an overwrite —
/// the stored copy may already hold landed sections.
pub async fn create_draft_run(
    s3: &S3Client,
    bucket: &str,
    run: DraftRun,
) -> Result<LoadedRun, ReportStoreError> {
    run.validate()
        .map_err(|error| ReportStoreError::InvalidRun(error.to_string()))?;
    let key = claria_core::s3_keys::report_draft_run(run.client_id, run.report_id, run.run_id);
    match claria_storage::state::save_state_if_none_match(s3, bucket, &key, &run).await {
        Ok(etag) => Ok(LoadedRun {
            run,
            key,
            etag: require_etag(etag)?,
        }),
        Err(StorageError::PreconditionFailed { .. }) => Err(ReportStoreError::Conflict),
        Err(source) => Err(ReportStoreError::storage(
            "creating the drafting run",
            source,
        )),
    }
}

pub async fn load_draft_run(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    run_id: Uuid,
) -> Result<LoadedRun, ReportStoreError> {
    if client_id.is_nil() || report_id.is_nil() || run_id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "Client, report, and run IDs must not be nil.".to_string(),
        ));
    }
    let key = claria_core::s3_keys::report_draft_run(client_id, report_id, run_id);
    let loaded = load_existing(s3, bucket, &key).await?;
    if loaded.run.client_id != client_id
        || loaded.run.report_id != report_id
        || loaded.run.run_id != run_id
    {
        return Err(ReportStoreError::InvalidRun(
            "The drafting run does not match its storage key.".to_string(),
        ));
    }
    Ok(loaded)
}

/// Validate and conditionally write a loaded run back to its key, stamping
/// `updated_at` and refreshing the ETag. A concurrent write elsewhere surfaces
/// as [`ReportStoreError::Conflict`] and leaves the stored run untouched.
pub async fn save_draft_run(
    s3: &S3Client,
    bucket: &str,
    loaded: &mut LoadedRun,
) -> Result<(), ReportStoreError> {
    loaded.run.updated_at = jiff::Timestamp::now();
    loaded
        .run
        .validate()
        .map_err(|error| ReportStoreError::InvalidRun(error.to_string()))?;
    let etag = claria_storage::state::save_state_if_match(
        s3,
        bucket,
        &loaded.key,
        &loaded.run,
        &loaded.etag,
    )
    .await
    .map_err(|source| match source {
        StorageError::PreconditionFailed { .. } => ReportStoreError::Conflict,
        other => ReportStoreError::storage("saving the drafting run", other),
    })?;
    loaded.etag = require_etag(etag)?;
    Ok(())
}

/// Every drafting run recorded for one Writing session, newest first.
pub async fn list_draft_runs(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<Vec<DraftRun>, ReportStoreError> {
    if client_id.is_nil() || report_id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "Client and report IDs must not be nil.".to_string(),
        ));
    }
    let keys = claria_storage::objects::list_objects(
        s3,
        bucket,
        &claria_core::s3_keys::report_draft_runs_prefix(client_id, report_id),
    )
    .await
    .map_err(|source| ReportStoreError::storage("listing drafting runs", source))?;

    use futures::StreamExt;
    // The key is moved into each future rather than borrowed from `keys`: an
    // async block that borrows its closure argument is only higher-ranked over
    // that lifetime by inference, and the inference fails once this call sits
    // under a Tauri command's own lifetime quantification.
    let loads = keys.into_iter().map(|key| async move {
        load_existing(s3, bucket, &key)
            .await
            .map(|loaded| loaded.run)
    });
    let mut runs = futures::stream::iter(loads)
        .buffered(claria_storage::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    runs.sort_by_key(|run| std::cmp::Reverse(run.updated_at));
    Ok(runs)
}

async fn load_existing(
    s3: &S3Client,
    bucket: &str,
    key: &str,
) -> Result<LoadedRun, ReportStoreError> {
    let output = claria_storage::objects::get_object(s3, bucket, key)
        .await
        .map_err(|source| match source {
            StorageError::NotFound { .. } => ReportStoreError::InvalidInput(
                "That drafting run is no longer available.".to_string(),
            ),
            other => ReportStoreError::storage("loading the drafting run", other),
        })?;
    let run = decode_draft_run(&output.body)
        .map_err(|error| ReportStoreError::InvalidRun(error.to_string()))?;
    Ok(LoadedRun {
        run,
        key: key.to_string(),
        etag: require_etag(output.etag.unwrap_or_default())?,
    })
}

fn require_etag(etag: String) -> Result<String, ReportStoreError> {
    if etag.trim().is_empty() {
        Err(ReportStoreError::InvalidRun(
            "S3 did not return an ETag for the drafting run.".to_string(),
        ))
    } else {
        Ok(etag)
    }
}
