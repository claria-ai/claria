//! The record inventory: the single S3 walk behind the sidecar-visibility
//! rules, plus record-text fetching and content search built on top of it.
//!
//! The visibility rules themselves are pure and live in
//! `claria_core::s3_keys::visible_record_files`; this module owns the listing
//! that feeds them and the cache-backed text reads that follow.

use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    S3_FETCH_CONCURRENCY,
    cache::RecordCache,
    error::{RecordsError, storage},
};

/// One user-visible record file and the object its readable text comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordInventoryEntry {
    /// Path relative to the client's `records/{uuid}/` prefix.
    pub filename: String,
    /// Key of the file itself.
    pub base_key: String,
    /// Key of the object holding the file's readable text: the `.text`
    /// sidecar when one exists, otherwise the file itself.
    pub source_key: String,
    /// Listed ETag of the source object (empty when S3 reported none, which
    /// never matches a cache entry — a safe miss).
    pub source_etag: String,
    /// Listed size of the source object in bytes.
    pub source_size: u64,
}

/// List a client's user-visible record files with their resolved text
/// sources, applying the sidecar-visibility rules exactly once.
pub async fn record_inventory(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
) -> Result<Vec<RecordInventoryEntry>, RecordsError> {
    let prefix = claria_core::s3_keys::client_records_prefix(client_id);
    let objects = claria_storage::objects::list_objects_with_metadata(s3, bucket, &prefix)
        .await
        .map_err(|source| storage("listing record files", source))?;
    let keys: Vec<&str> = objects.iter().map(|object| object.key.as_str()).collect();

    Ok(claria_core::s3_keys::visible_record_files(&prefix, &keys)
        .into_iter()
        .map(|file| {
            let source = &objects[file.source_index];
            RecordInventoryEntry {
                filename: file.filename,
                base_key: objects[file.base_index].key.clone(),
                source_key: source.key.clone(),
                source_etag: source.etag.clone().unwrap_or_default(),
                source_size: u64::try_from(source.size).unwrap_or(u64::MAX),
            }
        })
        .collect())
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
) -> Result<Vec<String>, RecordsError> {
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
) -> Result<Vec<(String, Option<String>)>, RecordsError> {
    let inventory = record_inventory(s3, bucket, client_id).await?;

    // Content validation after the GET is authoritative; filenames never
    // decide whether structured text is usable. One unreadable file degrades
    // to `None` (logged) instead of failing the whole listing.
    let fetches = inventory.into_iter().map(|entry| async move {
        let text = fetch_entry_text(s3, bucket, cache, &entry)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "skipping unreadable record file text");
                None
            });
        (entry.filename, text)
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
/// `filename` must name a user-visible file; a hidden sidecar or chat-history
/// entry resolves to `None`.
pub async fn fetch_record_text(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: uuid::Uuid,
    filename: &str,
    cache: &RecordCache,
) -> Result<Option<String>, RecordsError> {
    let inventory = record_inventory(s3, bucket, client_id).await?;
    let Some(entry) = inventory
        .into_iter()
        .find(|entry| entry.filename == filename)
    else {
        return Ok(None);
    };
    fetch_entry_text(s3, bucket, cache, &entry).await
}

/// Read one inventory entry's text through the cache. `Ok(None)` means the
/// source is too large or not printable text; a read failure is an error.
async fn fetch_entry_text(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    cache: &RecordCache,
    entry: &RecordInventoryEntry,
) -> Result<Option<String>, RecordsError> {
    if entry.source_size > claria_core::record_text::MAX_RECORD_TEXT_BYTES {
        return Ok(None);
    }
    let body = cache
        .get_or_fetch(s3, bucket, &entry.source_key, &entry.source_etag)
        .await?;
    if u64::try_from(body.len()).unwrap_or(u64::MAX)
        > claria_core::record_text::MAX_RECORD_TEXT_BYTES
    {
        return Ok(None);
    }
    Ok(claria_core::record_text::decode_record_text(body.as_ref()).map(ToString::to_string))
}
