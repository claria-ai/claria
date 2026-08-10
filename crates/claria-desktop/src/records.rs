//! Client/record listing backed by S3: one ListObjectsV2, then
//! bounded-concurrent read-through cache fetches (serial GETs made the list
//! pages take seconds at ~100 records).
//!
//! The list response carries each object's ETag, which revalidates the cache:
//! a key whose listed ETag matches its cached entry is served without a GET.
//!
//! Concurrency is bounded with `buffered`, which caps in-flight fetches and
//! yields results in input order — so the `created_at` sort below and the
//! caller's listing order stay deterministic.

use std::collections::HashSet;

use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};

use crate::record_cache::RecordCache;

/// Max concurrent GetObject requests when hydrating a listing.
pub const S3_FETCH_CONCURRENCY: usize = 16;

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
pub fn validate_client_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Client name cannot be empty.".to_string());
    }
    if name.chars().count() > MAX_CLIENT_NAME_CHARS {
        return Err(format!(
            "Client name cannot exceed {MAX_CLIENT_NAME_CHARS} characters."
        ));
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
) -> Result<Vec<ClientSummary>, String> {
    let entries = claria_storage::objects::list_object_etags(
        s3,
        bucket,
        claria_core::s3_keys::CLIENTS_PREFIX,
    )
    .await
    .map_err(|e| e.to_string())?;

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
) -> Result<ClientRecordDetails, String> {
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
                .map_err(|error| error.to_string())
        },
        async {
            claria_storage::objects::list_objects_with_metadata(s3, bucket, &report_prefix)
                .await
                .map_err(|error| error.to_string())
        },
        async {
            claria_storage::objects::sum_object_version_sizes(s3, bucket, &records_prefix)
                .await
                .map_err(|error| error.to_string())
        },
        async {
            claria_storage::objects::sum_object_version_sizes(s3, bucket, &report_prefix)
                .await
                .map_err(|error| error.to_string())
        },
        load_client_name_history(s3, bucket, client_id),
    )?;

    let all_record_keys: HashSet<&str> = record_objects
        .iter()
        .map(|object| object.key.as_str())
        .collect();
    let file_count = record_objects
        .iter()
        .filter(|object| !claria_core::s3_keys::is_hidden_sidecar(&object.key, &all_record_keys))
        .filter_map(|object| object.key.strip_prefix(&records_prefix))
        .filter(|filename| !filename.is_empty() && !filename.starts_with("chat-history/"))
        .count() as u64;

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
) -> Result<Vec<ClientNameHistoryEntry>, String> {
    let key = claria_core::s3_keys::client(client_id);
    let versions = claria_storage::objects::list_object_versions(s3, bucket, &key)
        .await
        .map_err(|error| error.to_string())?;
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
                .map_err(|error| error.to_string())?;
                let client: claria_core::models::client::Client =
                    serde_json::from_slice(&output.body).map_err(|error| error.to_string())?;
                if client.id != client_id {
                    return Err(
                        "Client record identifier does not match its storage key.".to_string()
                    );
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
    let snapshots: Vec<Result<ClientNameHistoryEntry, String>> =
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
) -> Result<ClientNameUpdate, String> {
    let name = validate_client_name(name)?;
    let (mut client, etag) = load_client(s3, bucket, client_id).await?;
    if etag.is_empty() {
        return Err(
            "Client record is missing its S3 version identifier; reload and try again.".to_string(),
        );
    }

    let updated_at = jiff::Timestamp::now();
    client.name = name.clone();
    client.updated_at = updated_at;
    let body = serde_json::to_vec_pretty(&client).map_err(|error| error.to_string())?;
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
    .map_err(|error| match error {
        claria_storage::error::StorageError::PreconditionFailed { .. } => {
            "This client record changed on another computer. Reload it before renaming.".to_string()
        }
        other => other.to_string(),
    })?;

    Ok(ClientNameUpdate {
        id: client.id.to_string(),
        name,
        updated_at: updated_at.to_string(),
    })
}

async fn load_client(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
) -> Result<(claria_core::models::client::Client, String), String> {
    let key = claria_core::s3_keys::client(client_id);
    claria_storage::state::load_state_checked(
        s3,
        bucket,
        &key,
        None,
        |client: &claria_core::models::client::Client| {
            if client.id != client_id {
                return Err(
                    "Client record identifier does not match its storage key.".to_string()
                );
            }
            Ok(())
        },
    )
    .await
    .map_err(|error| match error {
        claria_storage::error::StorageError::InvalidState { reason, .. } => reason,
        other => other.to_string(),
    })
}

/// Filenames of record files whose readable text contains `query`,
/// case-insensitively. An empty or whitespace query matches nothing.
///
/// Goes through the same read-through cache as chat context, so repeating a
/// search over unchanged records costs one listing and zero GETs.
pub async fn search_record_contents(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
    cache: &RecordCache,
    query: &str,
) -> Result<Vec<String>, String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let texts = fetch_record_texts(s3, bucket, client_id, cache).await?;

    Ok(texts
        .into_iter()
        .filter(|(_, text)| {
            text.as_deref()
                .is_some_and(|t| t.to_lowercase().contains(&query))
        })
        .map(|(filename, _)| filename)
        .collect())
}

/// Fetch readable text for every record file under a client, in listing
/// order. Generated `.text` sidecars take precedence; otherwise the original
/// object is used when its bytes are printable UTF-8, regardless of extension.
/// Chat-history entries are skipped. `None` means the file is binary, too
/// large, missing, or has no readable extraction.
pub async fn fetch_record_texts(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
    cache: &RecordCache,
) -> Result<Vec<(String, Option<String>)>, String> {
    let prefix = claria_core::s3_keys::client_records_prefix(client_id);
    let objects = claria_storage::objects::list_objects_with_metadata(s3, bucket, &prefix)
        .await
        .map_err(|error| error.to_string())?;
    let all_keys: std::collections::HashSet<&str> =
        objects.iter().map(|object| object.key.as_str()).collect();
    let by_key: std::collections::HashMap<&str, &claria_storage::objects::ObjectMeta> = objects
        .iter()
        .map(|object| (object.key.as_str(), object))
        .collect();

    // Resolve each visible file to its generated extraction when one exists,
    // or to the original object otherwise. Content validation after the GET is
    // authoritative; filenames never decide whether structured text is usable.
    let targets: Vec<_> = objects
        .iter()
        .filter_map(|object| {
            if claria_core::s3_keys::is_hidden_sidecar(&object.key, &all_keys) {
                return None;
            }
            let filename = object.key.strip_prefix(&prefix)?;
            if filename.is_empty() || filename.starts_with("chat-history/") {
                return None;
            }

            let sidecar_key = format!("{}.text", object.key);
            let source = by_key.get(sidecar_key.as_str()).copied().unwrap_or(object);
            Some((
                filename.to_string(),
                source.key.clone(),
                source.etag.clone().unwrap_or_default(),
                u64::try_from(source.size).unwrap_or(u64::MAX),
            ))
        })
        .collect();

    let fetches = targets
        .into_iter()
        .map(|(filename, source_key, etag, source_size)| async move {
            let text = if source_size > claria_core::record_text::MAX_RECORD_TEXT_BYTES {
                None
            } else {
                match cache.get_or_fetch(s3, bucket, &source_key, &etag).await {
                    Ok(body)
                        if u64::try_from(body.len()).unwrap_or(u64::MAX)
                            <= claria_core::record_text::MAX_RECORD_TEXT_BYTES =>
                    {
                        claria_core::record_text::decode_record_text(body.as_ref())
                            .map(ToString::to_string)
                    }
                    Ok(_) | Err(_) => None,
                }
            };
            (filename, text)
        });

    Ok(futures::stream::iter(fetches)
        .buffered(S3_FETCH_CONCURRENCY)
        .collect()
        .await)
}

/// Fetch one record file's readable representation.
///
/// A generated extraction/transcript sidecar wins when present. Files without
/// sidecars are read directly and accepted based on their bytes, which keeps
/// JSON, Markdown, CSV, and extensionless text in their original formats.
pub async fn fetch_record_text(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
    filename: &str,
    cache: &RecordCache,
) -> Result<Option<String>, String> {
    let base_key = claria_core::s3_keys::client_record_file(client_id, filename);
    let sidecar_key = format!("{base_key}.text");
    let objects = claria_storage::objects::list_objects_with_metadata(s3, bucket, &base_key)
        .await
        .map_err(|error| error.to_string())?;
    let source = objects
        .iter()
        .find(|object| object.key == sidecar_key)
        .or_else(|| objects.iter().find(|object| object.key == base_key));
    let Some(source) = source else {
        return Ok(None);
    };
    if u64::try_from(source.size).unwrap_or(u64::MAX)
        > claria_core::record_text::MAX_RECORD_TEXT_BYTES
    {
        return Ok(None);
    }

    let body = cache
        .get_or_fetch(
            s3,
            bucket,
            &source.key,
            source.etag.as_deref().unwrap_or(""),
        )
        .await?;
    if u64::try_from(body.len()).unwrap_or(u64::MAX)
        > claria_core::record_text::MAX_RECORD_TEXT_BYTES
    {
        return Ok(None);
    }

    Ok(claria_core::record_text::decode_record_text(body.as_ref()).map(ToString::to_string))
}
