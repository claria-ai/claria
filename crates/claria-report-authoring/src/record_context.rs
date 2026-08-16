//! Record inventory and the whole-report readable-record snapshot.

use aws_sdk_s3::Client as S3Client;
use claria_bedrock::report;
use claria_core::models::report::ReportRecordContextFile;
use claria_storage::error::StorageError;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{FullRecordContextSummary, ReportAuthoringError, context::escape_delimiter_characters};

#[derive(Debug, Clone)]
pub(crate) struct RecordInventoryEntry {
    pub(crate) filename: String,
    pub(crate) read_key: String,
    pub(crate) source_bytes: u64,
}

/// The sidecar-visibility rules live in
/// `claria_core::s3_keys::visible_record_files`; this walk only feeds them the
/// listing. (`claria-records` owns the general S3-walking inventory, but this
/// crate cannot depend on it — the client lifecycle in `claria-records`
/// depends on this crate to restore report workspaces.)
pub(crate) async fn load_record_inventory(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<Vec<RecordInventoryEntry>, StorageError> {
    let prefix = claria_core::s3_keys::client_records_prefix(client_id);
    let objects = claria_storage::objects::list_objects_with_metadata(s3, bucket, &prefix).await?;
    let keys: Vec<&str> = objects.iter().map(|object| object.key.as_str()).collect();

    Ok(claria_core::s3_keys::visible_record_files(&prefix, &keys)
        .into_iter()
        .map(|file| {
            let source = &objects[file.source_index];
            RecordInventoryEntry {
                filename: file.filename,
                read_key: source.key.clone(),
                source_bytes: u64::try_from(source.size).unwrap_or(u64::MAX),
            }
        })
        .collect())
}

pub(crate) struct FullRecordContext {
    pub(crate) prompt: String,
    pub(crate) summary: FullRecordContextSummary,
    pub(crate) files: Vec<ReportRecordContextFile>,
}

enum LoadedFullRecord {
    Included {
        filename: String,
        text: String,
        sha256: String,
    },
    Unavailable {
        filename: String,
        reason: &'static str,
    },
}

/// Load an all-or-nothing snapshot of every readable record representation.
/// The metadata preflight caps downloads to the selected model's approximate
/// input capacity; the exact CountTokens check still runs on the complete
/// Bedrock request before inference.
pub(crate) async fn load_full_record_context(
    s3: &S3Client,
    bucket: &str,
    model_id: &str,
    inventory: &[RecordInventoryEntry],
) -> Result<FullRecordContext, ReportAuthoringError> {
    use futures::stream::{self, StreamExt};

    // Three source bytes per available input token leaves headroom for the
    // fixed tool schemas, current report/template, and retained conversation.
    // It is deliberately conservative because UTF-8 and JSON escaping can
    // expand before the provider tokenizer sees the request.
    let max_source_bytes = u64::from(report::report_input_token_budget(model_id)).saturating_mul(3);
    let eligible_source_bytes = inventory
        .iter()
        .filter(|entry| entry.source_bytes <= claria_core::record_text::MAX_RECORD_TEXT_BYTES)
        .map(|entry| entry.source_bytes)
        .fold(0_u64, u64::saturating_add);
    if eligible_source_bytes > max_source_bytes {
        return Err(ReportAuthoringError::InvalidInput(format!(
            "The readable client-record snapshot is too large to place in this model's initial context ({eligible_source_bytes} bytes; safe limit {max_source_bytes}). Remove or split oversized records before generating the whole report."
        )));
    }

    let fetches = inventory.iter().cloned().map(|entry| async move {
        if entry.source_bytes > claria_core::record_text::MAX_RECORD_TEXT_BYTES {
            return Ok(LoadedFullRecord::Unavailable {
                filename: entry.filename.clone(),
                reason: "source_too_large",
            });
        }
        let output = claria_storage::objects::get_object_bounded(
            s3,
            bucket,
            &entry.read_key,
            claria_core::record_text::MAX_RECORD_TEXT_BYTES,
        )
        .await?;
        let Some(text) = claria_core::record_text::decode_record_text(&output.body) else {
            return Ok(LoadedFullRecord::Unavailable {
                filename: entry.filename.clone(),
                reason: "text_extraction_unavailable",
            });
        };
        let sha256 = Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok::<LoadedFullRecord, StorageError>(LoadedFullRecord::Included {
            filename: entry.filename.clone(),
            text: text.to_string(),
            sha256,
        })
    });
    let loaded = stream::iter(fetches)
        .buffered(claria_storage::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut files = Vec::new();
    let mut unavailable = Vec::new();
    let mut record_context_files = Vec::new();
    let mut total_characters = 0_u64;
    for record in loaded {
        match record.map_err(|source| {
            ReportAuthoringError::storage("reading the full-draft record snapshot", source)
        })? {
            LoadedFullRecord::Included {
                filename,
                text,
                sha256,
            } => {
                let characters = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
                total_characters = total_characters.saturating_add(characters);
                record_context_files.push(ReportRecordContextFile::Included {
                    filename: filename.clone(),
                    sha256: sha256.clone(),
                    characters,
                });
                files.push(serde_json::json!({
                    "filename": filename,
                    "sha256": sha256,
                    "text": text
                }));
            }
            LoadedFullRecord::Unavailable { filename, reason } => {
                record_context_files.push(ReportRecordContextFile::Unavailable {
                    filename: filename.clone(),
                    reason: reason.to_string(),
                });
                unavailable.push(serde_json::json!({
                    "filename": filename,
                    "reason": reason
                }));
            }
        }
    }
    if files.is_empty() {
        return Err(ReportAuthoringError::InvalidInput(
            "No readable client-record text is available. Generate text extraction for at least one record before filling the whole report."
                .to_string(),
        ));
    }

    let included_files = u32::try_from(files.len()).unwrap_or(u32::MAX);
    let unavailable_files = u32::try_from(unavailable.len()).unwrap_or(u32::MAX);
    // Deliberately compact, unlike the pretty-printed report context: this
    // snapshot is dominated by record prose where indentation adds no
    // structure signal, and it is the largest payload the writer ever sends.
    let json = serde_json::to_string(&serde_json::json!({
        "snapshot_complete": unavailable_files == 0,
        "files": files,
        "unavailable_files": unavailable
    }))
    .map_err(|_| {
        ReportAuthoringError::InvalidInput(
            "Claria could not serialize the readable-record snapshot.".to_string(),
        )
    })?;
    Ok(FullRecordContext {
        prompt: format!(
            "<untrusted_record_context>{}</untrusted_record_context>",
            escape_delimiter_characters(&json)
        ),
        summary: FullRecordContextSummary {
            included_files,
            unavailable_files,
            total_characters,
        },
        files: record_context_files,
    })
}
