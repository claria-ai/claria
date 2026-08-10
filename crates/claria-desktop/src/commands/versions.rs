//! Version-history commands — the S3 versioning surface for record files
//! and deleted-item recovery.

use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandContext, CommandError, parse_uuid, run};
use crate::state::DesktopState;

/// A single version of a file in a client's record.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FileVersion {
    pub version_id: String,
    pub size: i32,
    pub last_modified: Option<String>,
    pub is_latest: bool,
}

/// A file that has been deleted (has a delete marker as the latest version).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DeletedFile {
    pub filename: String,
    pub deleted_at: Option<String>,
    pub version_id: String,
}

/// A client that has been deleted (has a delete marker on the client JSON).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DeletedClient {
    pub id: String,
    pub name: String,
    pub deleted_at: Option<String>,
    pub version_id: String,
}

/// List all versions of a specific file in a client's record.
#[tauri::command]
#[specta::specta]
pub async fn list_file_versions(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<Vec<FileVersion>, String> {
    run("list_file_versions", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let key = claria_core::s3_keys::client_record_file(id, &filename);

        let versions =
            claria_storage::objects::list_object_versions(&ctx.s3, &ctx.bucket, &key).await?;

        Ok(versions
            .into_iter()
            .filter(|v| !v.is_delete_marker)
            .map(|v| FileVersion {
                version_id: v.version_id,
                size: v.size as i32,
                last_modified: v.last_modified,
                is_latest: v.is_latest,
            })
            .collect())
    })
    .await
}

/// Get the text content of a specific version of a file.
#[tauri::command]
#[specta::specta]
pub async fn get_file_version_text(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    version_id: String,
) -> Result<String, String> {
    run("get_file_version_text", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let key = claria_core::s3_keys::client_record_file(id, &filename);

        let output =
            claria_storage::objects::get_object_version(&ctx.s3, &ctx.bucket, &key, &version_id)
                .await?;

        String::from_utf8(output.body).map_err(|e| CommandError::Msg(e.to_string()))
    })
    .await
}

/// Restore a previous version of a file by copying its content to a new PUT.
#[tauri::command]
#[specta::specta]
pub async fn restore_file_version(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    version_id: String,
) -> Result<(), String> {
    run("restore_file_version", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let key = claria_core::s3_keys::client_record_file(id, &filename);

        // Fetch the old version's content.
        let output =
            claria_storage::objects::get_object_version(&ctx.s3, &ctx.bucket, &key, &version_id)
                .await?;

        // Write it back as the current version.
        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            &key,
            output.body,
            output.content_type.as_deref(),
        )
        .await?;

        tracing::info!(client_id = %id, filename, version_id, "file version restored");

        Ok(())
    })
    .await
}

/// List deleted files in a client's record (files with a delete marker).
#[tauri::command]
#[specta::specta]
pub async fn list_deleted_files(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<DeletedFile>, String> {
    run("list_deleted_files", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let prefix = claria_core::s3_keys::client_records_prefix(id);

        let deleted =
            claria_storage::objects::list_deleted_objects(&ctx.s3, &ctx.bucket, &prefix).await?;

        // Collect all deleted keys so we can hide sidecar `.text` files.
        let entries: Vec<_> = deleted
            .iter()
            .filter_map(|d| {
                let filename = d.key.strip_prefix(&prefix)?;
                if filename.is_empty() {
                    return None;
                }
                Some(filename.to_string())
            })
            .collect();
        let all_deleted: std::collections::HashSet<&str> =
            entries.iter().map(|s| s.as_str()).collect();

        Ok(deleted
            .into_iter()
            .filter_map(|d| {
                let filename = d.key.strip_prefix(&prefix)?.to_string();
                if filename.is_empty() {
                    return None;
                }
                // Hide sidecar files whose base file also has a delete marker —
                // the same rule the live listing applies via `is_hidden_sidecar`,
                // evaluated here over delete markers instead of current objects.
                if claria_core::s3_keys::is_hidden_sidecar(&filename, &all_deleted) {
                    return None;
                }
                Some(DeletedFile {
                    filename,
                    deleted_at: d.last_modified,
                    version_id: d.version_id,
                })
            })
            .collect())
    })
    .await
}

/// Restore a deleted file by re-putting the most recent real version as a new version.
///
/// This preserves the full version history (including the delete marker) for
/// HIPAA audit-trail compliance, instead of removing the delete marker.
#[tauri::command]
#[specta::specta]
pub async fn restore_deleted_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    version_id: String,
) -> Result<(), String> {
    let _ = version_id; // kept for API compatibility; we find the latest real version ourselves
    run("restore_deleted_file", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let key = claria_core::s3_keys::client_record_file(id, &filename);

        // Find the most recent non-delete-marker version.
        let versions =
            claria_storage::objects::list_object_versions(&ctx.s3, &ctx.bucket, &key).await?;
        let real = versions
            .iter()
            .find(|v| !v.is_delete_marker)
            .ok_or_else(|| CommandError::Msg(format!("no restorable version found for {key}")))?;

        // Fetch that version's content and write it back as a new current version.
        let output = claria_storage::objects::get_object_version(
            &ctx.s3,
            &ctx.bucket,
            &key,
            &real.version_id,
        )
        .await?;

        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            &key,
            output.body,
            output.content_type.as_deref(),
        )
        .await?;

        tracing::info!(client_id = %id, filename, "deleted file restored");

        Ok(())
    })
    .await
}

/// List deleted clients (client JSON files with a delete marker).
#[tauri::command]
#[specta::specta]
pub async fn list_deleted_clients(
    state: State<'_, DesktopState>,
) -> Result<Vec<DeletedClient>, String> {
    run("list_deleted_clients", async {
        let ctx = CommandContext::new(&state).await?;

        let deleted = claria_storage::objects::list_deleted_objects(
            &ctx.s3,
            &ctx.bucket,
            claria_core::s3_keys::CLIENTS_PREFIX,
        )
        .await?;

        // Each deleted client costs two round-trips (ListObjectVersions, then
        // GetObject on the surviving version), which ran serially. `buffered`
        // caps in-flight requests and yields results in input order, so the
        // returned listing keeps the order `list_deleted_objects` produced.
        // The futures are collected up front so the stream borrows them with a
        // concrete lifetime — mapping references straight into `buffered` trips
        // a higher-ranked-lifetime error.
        let lookups: Vec<_> = deleted
            .iter()
            .map(|d| deleted_client_name(&ctx.s3, &ctx.bucket, &d.key))
            .collect();
        let names: Vec<Result<String, CommandError>> = futures::stream::iter(lookups)
            .buffered(claria_records::S3_FETCH_CONCURRENCY)
            .collect()
            .await;

        let mut clients = Vec::with_capacity(deleted.len());
        for (d, name) in deleted.iter().zip(names) {
            // Extract the UUID from the key (e.g. "clients/abc-123.json" → "abc-123")
            let id = d
                .key
                .strip_prefix(claria_core::s3_keys::CLIENTS_PREFIX)
                .and_then(|s| s.strip_suffix(".json"))
                .unwrap_or(&d.key)
                .to_string();

            clients.push(DeletedClient {
                id,
                name: name?,
                deleted_at: d.last_modified.clone(),
                version_id: d.version_id.clone(),
            });
        }

        Ok(clients)
    })
    .await
}

/// Name shown for a deleted client whose JSON can't be read back.
const UNKNOWN_CLIENT_NAME: &str = "Unknown";

/// Resolve a deleted client's display name from its most recent
/// non-delete-marker version.
///
/// Only the version listing is fatal. A missing, unreadable, or unparseable
/// body degrades to [`UNKNOWN_CLIENT_NAME`] so one corrupt object doesn't
/// blank the whole deleted-items list.
async fn deleted_client_name(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<String, CommandError> {
    let versions = claria_storage::objects::list_object_versions(s3, bucket, key).await?;

    let Some(version) = versions.iter().find(|v| !v.is_delete_marker) else {
        tracing::warn!(
            key,
            version_count = versions.len(),
            "no non-delete-marker version found for deleted client"
        );
        return Ok(UNKNOWN_CLIENT_NAME.to_string());
    };

    if version.version_id.is_empty() {
        tracing::warn!(
            key,
            "deleted client has empty version_id (pre-versioning object)"
        );
        return Ok(UNKNOWN_CLIENT_NAME.to_string());
    }

    let version_id = &version.version_id;
    let output = match claria_storage::objects::get_object_version(s3, bucket, key, version_id)
        .await
    {
        Ok(output) => output,
        Err(e) => {
            tracing::warn!(key, version_id, error = %e, "failed to fetch deleted client version");
            return Ok(UNKNOWN_CLIENT_NAME.to_string());
        }
    };

    match serde_json::from_slice::<claria_core::models::client::Client>(&output.body) {
        Ok(client) => Ok(client.name),
        Err(e) => {
            tracing::warn!(key, version_id, error = %e, "failed to deserialize deleted client JSON");
            Ok(UNKNOWN_CLIENT_NAME.to_string())
        }
    }
}

/// Restore a deleted client by re-putting the most recent real version as a new version.
///
/// This preserves the full version history (including the delete marker) for
/// HIPAA audit-trail compliance, instead of removing the delete marker.
#[tauri::command]
#[specta::specta]
pub async fn restore_client(
    state: State<'_, DesktopState>,
    client_id: String,
    version_id: String,
) -> Result<(), String> {
    let _ = version_id; // kept for API compatibility; we find the latest real version ourselves
    run("restore_client", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let outcome = claria_records::restore_client(&ctx.s3, &ctx.bucket, id).await?;

        tracing::info!(
            client_id = %id,
            report_restored = outcome.report_restored,
            "deleted client restored"
        );

        Ok(())
    })
    .await
}
