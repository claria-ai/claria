//! Client/record listing backed by S3: one ListObjectsV2, then
//! bounded-concurrent GetObject calls (serial GETs made the list pages
//! take seconds at ~100 records).
//!
//! Concurrency is bounded with `buffered`, which caps in-flight GETs and
//! yields results in input order — so the `created_at` sort below and the
//! caller's listing order stay deterministic.

use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};

/// Max concurrent GetObject requests when hydrating a listing.
pub const S3_FETCH_CONCURRENCY: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ClientSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// Load all `clients/{id}.json` objects and return summaries sorted by
/// most recently created first. Unreadable or unparseable objects are
/// skipped with a warning.
pub async fn list_client_summaries(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
) -> Result<Vec<ClientSummary>, String> {
    let keys =
        claria_storage::objects::list_objects(s3, bucket, claria_core::s3_keys::CLIENTS_PREFIX)
            .await
            .map_err(|e| e.to_string())?;

    // Bounded-concurrent GETs that come back in listing order, so ties in
    // the created_at sort below stay deterministic. The futures are
    // collected up front so the stream borrows them with a concrete
    // lifetime — mapping references straight into `buffered` trips a
    // higher-ranked-lifetime error.
    let fetches: Vec<_> = keys
        .iter()
        .map(|key| claria_storage::objects::get_object(s3, bucket, key))
        .collect();
    let results: Vec<_> = futures::stream::iter(fetches)
        .buffered(S3_FETCH_CONCURRENCY)
        .collect()
        .await;

    let mut clients: Vec<ClientSummary> = Vec::with_capacity(results.len());

    for (key, output) in keys.iter().zip(results) {
        let output = match output {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(key, error = %e, "skipping unreadable client object");
                continue;
            }
        };

        let client: claria_core::models::client::Client = match serde_json::from_slice(&output.body)
        {
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

/// Fetch readable text for every record file under a client, in listing
/// order. `.text` sidecars are read via their parent file; chat-history
/// entries are skipped. `None` means the file has no readable text.
pub async fn fetch_record_texts(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
) -> Result<Vec<(String, Option<String>)>, String> {
    let prefix = claria_core::s3_keys::client_records_prefix(client_id);

    let keys = claria_storage::objects::list_objects(s3, bucket, &prefix)
        .await
        .map_err(|e| e.to_string())?;

    // Collect all keys into a set so we can identify sidecar files.
    let all_keys: std::collections::HashSet<&str> = keys.iter().map(|k| k.as_str()).collect();

    // Resolve (filename, key to GET) pairs up front, before going concurrent.
    let targets: Vec<(String, String)> = keys
        .iter()
        .filter_map(|key| {
            // Skip sidecar `.text` files — we read them via their parent.
            if let Some(base) = key.strip_suffix(".text")
                && all_keys.contains(base)
            {
                return None;
            }

            let filename = match key.strip_prefix(&prefix) {
                Some(f) if !f.is_empty() => f,
                _ => return None,
            };

            // Skip chat-history entries — they're not record context.
            if filename.starts_with("chat-history/") {
                return None;
            }

            // Plain text is read directly; other files via their `.text` sidecar.
            let fetch_key = if filename.ends_with(".txt") {
                key.clone()
            } else {
                format!("{key}.text")
            };

            Some((filename.to_string(), fetch_key))
        })
        .collect();

    let fetches: Vec<_> = targets
        .iter()
        .map(|(filename, fetch_key)| async move {
            let text = match claria_storage::objects::get_object(s3, bucket, fetch_key).await {
                Ok(output) => String::from_utf8(output.body).ok(),
                Err(_) => None,
            };
            (filename.clone(), text)
        })
        .collect();
    let results = futures::stream::iter(fetches)
        .buffered(S3_FETCH_CONCURRENCY)
        .collect()
        .await;

    Ok(results)
}
