//! Client CRUD backed by S3: one ListObjectsV2, then bounded-concurrent
//! read-through cache fetches (serial GETs made the list pages take seconds
//! at ~100 records).
//!
//! The list response carries each object's ETag, which revalidates the cache:
//! a key whose listed ETag matches its cached entry is served without a GET.
//!
//! Concurrency is bounded with `buffered`, which caps in-flight fetches and
//! yields results in input order — so the `created_at` sort below and the
//! caller's listing order stay deterministic.

use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    S3_FETCH_CONCURRENCY,
    cache::RecordCache,
    error::{RecordsError, storage},
};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClientSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClientNameUpdate {
    pub id: String,
    pub name: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClientNameHistoryEntry {
    pub name: String,
    pub changed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClientRecordDetails {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    // User-visible files, excluding extraction sidecars and Chat history.
    pub file_count: u64,
    // Current record and Writing objects, excluding historical S3 versions.
    pub storage_bytes: u64,
    // Current and noncurrent record and Writing object versions.
    pub storage_bytes_with_history: u64,
    pub name_history: Vec<ClientNameHistoryEntry>,
}

const MAX_CLIENT_NAME_CHARS: usize = 200;

/// Trim and validate a client name before it is persisted.
pub fn validate_client_name(name: &str) -> Result<String, RecordsError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(RecordsError::EmptyClientName);
    }
    if name.chars().count() > MAX_CLIENT_NAME_CHARS {
        return Err(RecordsError::ClientNameTooLong(MAX_CLIENT_NAME_CHARS));
    }
    Ok(name.to_string())
}

/// Load all `clients/{id}.json` objects and return summaries sorted by
/// most recently created first. Unreadable or unparseable objects are
/// skipped with a warning.
pub async fn list_client_summaries(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    cache: &RecordCache,
) -> Result<Vec<ClientSummary>, RecordsError> {
    let entries = claria_storage::objects::list_object_etags(
        s3,
        bucket,
        claria_core::s3_keys::CLIENTS_PREFIX,
    )
    .await
    .map_err(|source| storage("listing clients", source))?;

    // Bounded-concurrent read-through fetches that come back in listing order,
    // so ties in the created_at sort below stay deterministic. Each key is
    // served from cache when its listed ETag matches, otherwise GET+repopulate.
    // The futures are collected up front so the stream borrows them with a
    // concrete lifetime — mapping references straight into `buffered` trips a
    // higher-ranked-lifetime error.
    let fetches: Vec<_> = entries
        .iter()
        .map(|(key, etag)| cache.get_or_fetch(s3, bucket, key, etag))
        .collect();
    let results: Vec<_> = futures::stream::iter(fetches)
        .buffered(S3_FETCH_CONCURRENCY)
        .collect()
        .await;

    let mut clients: Vec<ClientSummary> = Vec::with_capacity(results.len());

    for ((key, _etag), body) in entries.iter().zip(results) {
        let body = match body {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(key, error = %e, "skipping unreadable client object");
                continue;
            }
        };

        let client: claria_core::models::client::Client = match serde_json::from_slice(&body) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(key, error = %e, "skipping unparseable client object");
                continue;
            }
        };

        clients.push(ClientSummary {
            id: client.id.to_string(),
            name: client.name,
            created_at: client.created_at.to_string(),
        });
    }

    // Sort by created_at descending (most recent first).
    clients.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(clients)
}

/// Load editable client metadata, storage statistics, and name history.
pub async fn get_client_record_details(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
) -> Result<ClientRecordDetails, RecordsError> {
    let (client, _) = load_client(s3, bucket, client_id).await?;
    let records_prefix = claria_core::s3_keys::client_records_prefix(client_id);
    let report_prefix = claria_core::s3_keys::report_authoring_client_prefix(client_id);
    let (
        record_objects,
        report_objects,
        record_storage_with_history,
        report_storage_with_history,
        name_history,
    ) = tokio::try_join!(
        async {
            claria_storage::objects::list_objects_with_metadata(s3, bucket, &records_prefix)
                .await
                .map_err(|source| storage("listing record files", source))
        },
        async {
            claria_storage::objects::list_objects_with_metadata(s3, bucket, &report_prefix)
                .await
                .map_err(|source| storage("listing report files", source))
        },
        async {
            claria_storage::objects::sum_object_version_sizes(s3, bucket, &records_prefix)
                .await
                .map_err(|source| storage("measuring record history", source))
        },
        async {
            claria_storage::objects::sum_object_version_sizes(s3, bucket, &report_prefix)
                .await
                .map_err(|source| storage("measuring report history", source))
        },
        load_client_name_history(s3, bucket, client_id),
    )?;

    let record_keys: Vec<&str> = record_objects
        .iter()
        .map(|object| object.key.as_str())
        .collect();
    let file_count =
        claria_core::s3_keys::visible_record_files(&records_prefix, &record_keys).len() as u64;

    let storage_bytes = record_objects
        .iter()
        .chain(report_objects.iter())
        .filter_map(|object| u64::try_from(object.size).ok())
        .fold(0_u64, u64::saturating_add);
    let storage_bytes_with_history =
        record_storage_with_history.saturating_add(report_storage_with_history);

    Ok(ClientRecordDetails {
        id: client.id.to_string(),
        name: client.name,
        created_at: client.created_at.to_string(),
        updated_at: client.updated_at.to_string(),
        file_count,
        storage_bytes,
        storage_bytes_with_history,
        name_history,
    })
}

async fn load_client_name_history(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
) -> Result<Vec<ClientNameHistoryEntry>, RecordsError> {
    let key = claria_core::s3_keys::client(client_id);
    let versions = claria_storage::objects::list_object_versions(s3, bucket, &key)
        .await
        .map_err(|source| storage("listing client versions", source))?;
    let fetches: Vec<_> = versions
        .into_iter()
        .filter(|version| !version.is_delete_marker)
        .map(|version| {
            let key = key.clone();
            async move {
                let output = claria_storage::objects::get_object_version(
                    s3,
                    bucket,
                    &key,
                    &version.version_id,
                )
                .await
                .map_err(|source| storage("reading a client version", source))?;
                let client: claria_core::models::client::Client =
                    serde_json::from_slice(&output.body)?;
                if client.id != client_id {
                    return Err(RecordsError::InvalidClient);
                }
                Ok(ClientNameHistoryEntry {
                    name: client.name,
                    changed_at: version
                        .last_modified
                        .unwrap_or_else(|| client.updated_at.to_string()),
                })
            }
        })
        .collect();
    let snapshots: Vec<Result<ClientNameHistoryEntry, RecordsError>> =
        futures::stream::iter(fetches)
            .buffered(S3_FETCH_CONCURRENCY)
            .collect()
            .await;

    // S3 returns newest-first. Walk chronologically so repeated metadata
    // versions with the same name collapse to the time that name first appeared.
    let mut history = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots.into_iter().rev() {
        let entry = snapshot?;
        if history
            .last()
            .is_none_or(|previous: &ClientNameHistoryEntry| previous.name != entry.name)
        {
            history.push(entry);
        }
    }
    history.reverse();
    Ok(history)
}

/// Rename a client without overwriting a concurrent metadata change.
pub async fn update_client_name(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
    name: &str,
) -> Result<ClientNameUpdate, RecordsError> {
    let name = validate_client_name(name)?;
    let (mut client, etag) = load_client(s3, bucket, client_id).await?;
    if etag.is_empty() {
        return Err(RecordsError::MissingConcurrencyToken);
    }

    let updated_at = jiff::Timestamp::now();
    client.name = name.clone();
    client.updated_at = updated_at;
    let body = serde_json::to_vec_pretty(&client)?;
    let key = claria_core::s3_keys::client(client_id);
    claria_storage::objects::put_object_if_match(
        s3,
        bucket,
        &key,
        body,
        Some("application/json"),
        &etag,
    )
    .await
    .map_err(|source| match source {
        claria_storage::error::StorageError::PreconditionFailed { .. } => {
            RecordsError::ConcurrentClientEdit
        }
        other => storage("renaming the client", other),
    })?;

    Ok(ClientNameUpdate {
        id: client.id.to_string(),
        name,
        updated_at: updated_at.to_string(),
    })
}

/// Load and validate the current client record, returning it with its ETag.
pub(crate) async fn load_client(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
) -> Result<(claria_core::models::client::Client, String), RecordsError> {
    let key = claria_core::s3_keys::client(client_id);
    claria_storage::state::load_state_checked(
        s3,
        bucket,
        &key,
        None,
        |client: &claria_core::models::client::Client| {
            crate::lifecycle::check_client_identity(client, client_id)
        },
    )
    .await
    .map_err(|source| match source {
        claria_storage::error::StorageError::InvalidState { .. }
        | claria_storage::error::StorageError::Serialization(_) => RecordsError::InvalidClient,
        other => storage("loading the client", other),
    })
}
