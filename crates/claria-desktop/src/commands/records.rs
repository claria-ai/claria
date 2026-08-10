//! Record file commands — files attached to a client record.

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{
    CommandContext, CommandError, parse_uuid, prompts::load_prompt, run,
    transcribe::{TranscribeOptionsOverrides, build_transcribe_options, maybe_translate},
    usage_audit_details,
};
use crate::state::DesktopState;

/// A file in a client's record (S3 object metadata).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RecordFile {
    pub filename: String,
    pub size: i32,
    pub uploaded_at: Option<String>,
}

/// The Bedrock model ID used for document text extraction.
///
/// Uses a Claude Sonnet inference profile — good quality at lower cost.
pub(crate) const EXTRACTION_MODEL_ID: &str = "us.anthropic.claude-sonnet-4-20250514-v1:0";

fn record_upload_content_type(extension: &str, bytes: &[u8]) -> Option<&'static str> {
    match extension {
        "pdf" => Some("application/pdf"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "doc" => Some("application/msword"),
        "json" => Some("application/json"),
        "jsonl" | "ndjson" => Some("application/x-ndjson"),
        "md" | "markdown" => Some("text/markdown; charset=utf-8"),
        "csv" => Some("text/csv; charset=utf-8"),
        "html" | "htm" => Some("text/html; charset=utf-8"),
        "xml" => Some("application/xml"),
        "yaml" | "yml" => Some("application/yaml"),
        "toml" => Some("application/toml"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "mp3" => Some("audio/mpeg"),
        "mp4" | "m4a" => Some("audio/mp4"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        "ogg" => Some("audio/ogg"),
        "amr" => Some("audio/amr"),
        "webm" => Some("audio/webm"),
        _ if claria_core::record_text::decode_record_text(bytes).is_some() => {
            Some("text/plain; charset=utf-8")
        }
        _ => None,
    }
}

/// List files in a client's record, excluding sidecar `.text` files.
///
/// `prefix` narrows the listing to filenames starting with it, mapped to the
/// S3 ListObjectsV2 `Prefix` parameter (`records/{id}/{prefix}`).
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id, count = tracing::field::Empty))]
pub async fn list_record_files(
    state: State<'_, DesktopState>,
    client_id: String,
    prefix: Option<String>,
) -> Result<Vec<RecordFile>, String> {
    run("list_record_files", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let records_prefix = claria_core::s3_keys::client_records_prefix(id);
        let list_prefix = match prefix.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => claria_core::s3_keys::client_records_search_prefix(id, p),
            None => records_prefix.clone(),
        };

        let objects =
            claria_storage::objects::list_objects_with_metadata(&ctx.s3, &ctx.bucket, &list_prefix)
                .await?;

        // Collect all keys into a set so we can check for base files when filtering sidecars.
        let all_keys: std::collections::HashSet<&str> =
            objects.iter().map(|o| o.key.as_str()).collect();

        let files: Vec<RecordFile> = objects
            .iter()
            .filter(|obj| !claria_core::s3_keys::is_hidden_sidecar(&obj.key, &all_keys))
            .filter_map(|obj| {
                // Strip the records prefix to get just the filename.
                let filename = obj.key.strip_prefix(&records_prefix)?;
                if filename.is_empty() {
                    return None;
                }
                Some(RecordFile {
                    filename: filename.to_string(),
                    size: obj.size as i32,
                    uploaded_at: obj.last_modified.clone(),
                })
            })
            .collect();

        tracing::Span::current().record("count", files.len() as u64);

        Ok(files)
    })
    .await
}

/// Filenames in a client's record whose readable text contains `query`,
/// case-insensitively.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id, count = tracing::field::Empty))]
pub async fn search_record_contents(
    state: State<'_, DesktopState>,
    client_id: String,
    query: String,
) -> Result<Vec<String>, String> {
    run("search_record_contents", async {
        let ctx = CommandContext::new(&state).await?;
        let id = parse_uuid(&client_id)?;

        let matches = claria_records::search_record_contents(
            &ctx.s3,
            &ctx.bucket,
            id,
            &state.record_cache,
            &query,
        )
        .await?;

        tracing::Span::current().record("count", matches.len() as u64);

        Ok(matches)
    })
    .await
}

/// Upload a file to a client's record from a local file path.
///
/// Printable UTF-8 files remain directly readable under their original names.
/// PDF and DOCX files receive a structured-Markdown `.text` sidecar via
/// Bedrock; audio files receive a transcript sidecar.
#[tauri::command]
#[specta::specta]
// Spans and log lines carry the client UUID, extension, and byte count —
// never the client-chosen filename, which is PHI.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(
        client_id = %client_id,
        extension = tracing::field::Empty,
        bytes = tracing::field::Empty
    )
)]
pub async fn upload_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    file_path: String,
) -> Result<RecordFile, String> {
    run("upload_record_file", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;

        let path = std::path::Path::new(&file_path);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| CommandError::Msg("Invalid file path".to_string()))?;

        let bytes = std::fs::read(path)
            .map_err(|e| CommandError::Msg(format!("Failed to read file: {e}")))?;
        let file_size = bytes.len() as i32;

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let span = tracing::Span::current();
        span.record("extension", extension.as_str());
        span.record("bytes", bytes.len() as u64);

        let content_type = record_upload_content_type(&extension, &bytes);

        // Upload the original file.
        let key = claria_core::s3_keys::client_record_file(id, filename);
        claria_storage::objects::put_object(&ctx.s3, &ctx.bucket, &key, bytes.clone(), content_type)
            .await?;

        tracing::info!(client_id = %id, extension, "record file uploaded");

        // The durable audit trail may carry the filename; it lives only in
        // access-controlled S3.
        ctx.record_audit(
            ctx.audit_event("record_file_uploaded", "record_file", filename)
                .with_details(serde_json::json!({
                    "client_id": id.to_string(),
                    "bytes": file_size,
                })),
        )
        .await;

        // Generate sidecar text extraction for supported document types.
        if let Some(format) = claria_bedrock::extract::document_format_for_extension(&extension) {
            let sidecar_key = format!("{key}.text");
            let extraction_prompt = load_prompt(&ctx.s3, &ctx.bucket, "pdf-extraction").await?;
            match claria_bedrock::extract::extract_document_text(
                &ctx.sdk_config,
                EXTRACTION_MODEL_ID,
                &bytes,
                filename,
                format,
                &extraction_prompt,
            )
            .await
            {
                Ok((text, usage)) => {
                    claria_storage::objects::put_object(
                        &ctx.s3,
                        &ctx.bucket,
                        &sidecar_key,
                        text.into_bytes(),
                        Some("text/markdown; charset=utf-8"),
                    )
                    .await?;

                    let mut audit_details =
                        usage_audit_details(EXTRACTION_MODEL_ID, usage.as_ref());
                    audit_details["client_id"] = serde_json::json!(id.to_string());
                    ctx.record_audit(
                        ctx.audit_event("extract_document_text", "record_file", filename)
                            .with_details(audit_details),
                    )
                    .await;

                    tracing::info!(client_id = %id, "sidecar text extraction uploaded");
                }
                Err(e) => {
                    // Non-fatal: the original file is already uploaded.
                    tracing::warn!(
                        client_id = %id,
                        error = %e,
                        "sidecar text extraction failed"
                    );
                }
            }
        } else if let Some(media_format) = claria_transcribe::media_format_for_extension(&extension)
        {
            let sidecar_key = format!("{key}.text");
            // Drag-drop uses saved preferences as-is. The wizard's separate
            // command (`upload_record_file_with_options`) is the override path.
            let options = build_transcribe_options(&ctx.cfg.transcription, None);
            let translate = ctx.cfg.transcription.translate_to_english;

            match claria_transcribe::transcribe_audio_with_options(
                &ctx.sdk_config,
                &ctx.bucket,
                &key,
                media_format,
                &options,
            )
            .await
            {
                Ok(mut result) => {
                    maybe_translate(&ctx, &mut result, translate).await;
                    let body = claria_transcribe::format_transcript_body(&result);
                    claria_storage::objects::put_object(
                        &ctx.s3,
                        &ctx.bucket,
                        &sidecar_key,
                        body.into_bytes(),
                        Some("text/plain"),
                    )
                    .await?;

                    tracing::info!(client_id = %id, "sidecar audio transcription uploaded");
                }
                Err(e) => {
                    // Non-fatal: the original file is already uploaded.
                    tracing::warn!(
                        client_id = %id,
                        error = %e,
                        "sidecar audio transcription failed"
                    );
                }
            }
        }

        Ok(RecordFile {
            filename: filename.to_string(),
            size: file_size,
            uploaded_at: Some(jiff::Timestamp::now().to_string()),
        })
    })
    .await
}

/// Upload an audio file and transcribe with the wizard's per-file options.
///
/// Mirrors `upload_record_file` but skips the legacy single-language path —
/// always goes through the new structured transcribe + optional Bedrock
/// translation. The `.text` sidecar contains the rendered headered body.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id))]
pub async fn upload_record_file_with_options(
    state: State<'_, DesktopState>,
    client_id: String,
    file_path: String,
    overrides: Option<TranscribeOptionsOverrides>,
) -> Result<RecordFile, String> {
    run("upload_record_file_with_options", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;

        let path = std::path::Path::new(&file_path);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| CommandError::Msg("Invalid file path".to_string()))?;
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let media_format = claria_transcribe::media_format_for_extension(&extension)
            .ok_or_else(|| CommandError::Msg(format!("Unsupported audio format: .{extension}")))?;

        let bytes = std::fs::read(path)
            .map_err(|e| CommandError::Msg(format!("Failed to read file: {e}")))?;
        let file_size = bytes.len() as i32;

        let content_type = record_upload_content_type(&extension, &bytes);

        let key = claria_core::s3_keys::client_record_file(id, filename);
        claria_storage::objects::put_object(&ctx.s3, &ctx.bucket, &key, bytes, content_type)
            .await?;
        tracing::info!(client_id = %id, extension, "record file uploaded (wizard path)");

        ctx.record_audit(
            ctx.audit_event("record_file_uploaded", "record_file", filename)
                .with_details(serde_json::json!({
                    "client_id": id.to_string(),
                    "bytes": file_size,
                })),
        )
        .await;

        let translate = overrides
            .as_ref()
            .and_then(|o| o.translate_to_english)
            .unwrap_or(ctx.cfg.transcription.translate_to_english);
        let options = build_transcribe_options(&ctx.cfg.transcription, overrides);

        let sidecar_key = format!("{key}.text");
        let mut result = claria_transcribe::transcribe_audio_with_options(
            &ctx.sdk_config,
            &ctx.bucket,
            &key,
            media_format,
            &options,
        )
        .await?;

        maybe_translate(&ctx, &mut result, translate).await;

        let body = claria_transcribe::format_transcript_body(&result);
        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            &sidecar_key,
            body.into_bytes(),
            Some("text/plain"),
        )
        .await?;

        tracing::info!(client_id = %id, "wizard transcription complete");

        Ok(RecordFile {
            filename: filename.to_string(),
            size: file_size,
            uploaded_at: Some(jiff::Timestamp::now().to_string()),
        })
    })
    .await
}

/// Save edits to the transcript sidecar. S3 versioning preserves every prior
/// body, including the Transcribe-generated v1 — clinicians restore any past
/// version (or the original) via the standard `list_file_versions` /
/// `restore_file_version` flow, which the frontend routes to the `.text`
/// sidecar for audio files.
#[tauri::command]
#[specta::specta]
pub async fn save_transcript_edits(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    body: String,
) -> Result<(), String> {
    run("save_transcript_edits", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let sidecar_key = format!(
            "{}.text",
            claria_core::s3_keys::client_record_file(id, &filename)
        );

        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            &sidecar_key,
            body.into_bytes(),
            Some("text/plain"),
        )
        .await?;

        ctx.record_audit(
            ctx.audit_event("save_transcript_edits", "transcript", &filename)
                .with_details(serde_json::json!({ "client_id": id.to_string() })),
        )
        .await;

        Ok(())
    })
    .await
}

/// Delete a file from a client's record, including its generated sidecar
/// when present.
#[tauri::command]
#[specta::specta]
pub async fn delete_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<(), String> {
    run("delete_record_file", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;

        let key = claria_core::s3_keys::client_record_file(id, &filename);

        // Delete the original file.
        claria_storage::objects::delete_object(&ctx.s3, &ctx.bucket, &key).await?;

        // Deleting a missing key in a versioned bucket creates a phantom delete
        // marker, so discover the optional sidecar before deleting it. This is
        // content-model agnostic: text files need no sidecar, while document and
        // audio derivatives are cleaned up without an extension allow-list.
        let sidecar_key = format!("{key}.text");
        match claria_storage::objects::list_objects(&ctx.s3, &ctx.bucket, &sidecar_key).await {
            Ok(keys) if keys.iter().any(|candidate| candidate == &sidecar_key) => {
                if let Err(error) =
                    claria_storage::objects::delete_object(&ctx.s3, &ctx.bucket, &sidecar_key).await
                {
                    tracing::warn!(
                        client_id = %id,
                        %error,
                        "failed to delete record text sidecar"
                    );
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    client_id = %id,
                    %error,
                    "failed to discover record text sidecar during delete"
                );
            }
        }

        tracing::info!(client_id = %id, "record file deleted");

        ctx.record_audit(
            ctx.audit_event("record_file_deleted", "record_file", &filename)
                .with_details(serde_json::json!({ "client_id": id.to_string() })),
        )
        .await;

        Ok(())
    })
    .await
}

/// Get the readable text for a record file.
///
/// Generated document/transcript sidecars take precedence. Otherwise any
/// printable UTF-8 original—including JSON, Markdown, CSV, and extensionless
/// text—is returned unchanged.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id))]
pub async fn get_record_file_text(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<String, String> {
    run("get_record_file_text", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;

        claria_records::fetch_record_text(&ctx.s3, &ctx.bucket, id, &filename, &state.record_cache)
            .await?
            .ok_or_else(|| {
                CommandError::Msg(
                    "No readable text is available. Upload printable UTF-8 text or extract a supported document or recording."
                        .to_string(),
                )
            })
    })
    .await
}

/// Create a plain text file in a client's record.
///
/// Writes the given content as a `.txt` file directly to S3. If the filename
/// doesn't already end in `.txt`, it is appended.
#[tauri::command]
#[specta::specta]
pub async fn create_text_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    content: String,
) -> Result<RecordFile, String> {
    run("create_text_record_file", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;

        // Ensure the filename ends with .txt.
        let filename = if filename.ends_with(".txt") {
            filename
        } else {
            format!("{filename}.txt")
        };

        let bytes = content.into_bytes();
        let file_size = bytes.len() as i32;

        let key = claria_core::s3_keys::client_record_file(id, &filename);
        claria_storage::objects::put_object(&ctx.s3, &ctx.bucket, &key, bytes, Some("text/plain"))
            .await?;

        tracing::info!(client_id = %id, "text record file created");

        ctx.record_audit(
            ctx.audit_event("record_file_created", "record_file", &filename)
                .with_details(serde_json::json!({
                    "client_id": id.to_string(),
                    "bytes": file_size,
                })),
        )
        .await;

        Ok(RecordFile {
            filename,
            size: file_size,
            uploaded_at: Some(jiff::Timestamp::now().to_string()),
        })
    })
    .await
}

/// Update the content of an existing plain text file in a client's record.
#[tauri::command]
#[specta::specta]
pub async fn update_text_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
    content: String,
) -> Result<(), String> {
    run("update_text_record_file", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;

        let key = claria_core::s3_keys::client_record_file(id, &filename);
        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            &key,
            content.into_bytes(),
            Some("text/plain"),
        )
        .await?;

        tracing::info!(client_id = %id, "text record file updated");

        ctx.record_audit(
            ctx.audit_event("record_file_updated", "record_file", &filename)
                .with_details(serde_json::json!({ "client_id": id.to_string() })),
        )
        .await;

        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Record context — text content for chat context injection
// ---------------------------------------------------------------------------

/// A record file with its readable text content, for chat context.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RecordContext {
    pub filename: String,
    pub text: String,
}

/// Load text content for all record files belonging to a client.
///
/// Printable UTF-8 originals are returned unchanged, regardless of extension.
/// PDF, DOCX, and audio files use their generated `.text` sidecars. Files with
/// no safe readable representation are omitted.
#[tauri::command]
#[specta::specta]
#[tracing::instrument(level = "trace", skip_all, fields(client_id = %client_id, files = tracing::field::Empty))]
pub async fn list_record_context(
    state: State<'_, DesktopState>,
    client_id: String,
) -> Result<Vec<RecordContext>, String> {
    run("list_record_context", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;

        let texts =
            claria_records::fetch_record_texts(&ctx.s3, &ctx.bucket, id, &state.record_cache)
                .await?;

        // Include all files — those without extracted text get an empty string
        // so the frontend can show them as context pills and offer re-extraction.
        let context_files: Vec<RecordContext> = texts
            .into_iter()
            .map(|(filename, text)| RecordContext {
                filename,
                text: text.unwrap_or_default(),
            })
            .collect();

        tracing::Span::current().record("files", context_files.len() as u64);

        Ok(context_files)
    })
    .await
}

/// Resolve readable text for a single record file.
///
/// PDF and DOCX files are re-extracted through Bedrock, audio is
/// retranscribed, and printable UTF-8 originals are returned unchanged.
#[tauri::command]
#[specta::specta]
pub async fn extract_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    filename: String,
) -> Result<RecordContext, String> {
    run("extract_record_file", async {
        let ctx = CommandContext::new(&state).await?;

        let id = parse_uuid(&client_id)?;
        let key = claria_core::s3_keys::client_record_file(id, &filename);

        let extension = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let sidecar_key = format!("{key}.text");

        let text = if let Some(format) =
            claria_bedrock::extract::document_format_for_extension(&extension)
        {
            // Document extraction (PDF, DOCX).
            let output = claria_storage::objects::get_object(&ctx.s3, &ctx.bucket, &key).await?;
            let extraction_prompt = load_prompt(&ctx.s3, &ctx.bucket, "pdf-extraction").await?;
            let (text, usage) = claria_bedrock::extract::extract_document_text(
                &ctx.sdk_config,
                EXTRACTION_MODEL_ID,
                &output.body,
                &filename,
                format,
                &extraction_prompt,
            )
            .await?;

            claria_storage::objects::put_object(
                &ctx.s3,
                &ctx.bucket,
                &sidecar_key,
                text.clone().into_bytes(),
                Some("text/markdown; charset=utf-8"),
            )
            .await?;

            let mut audit_details = usage_audit_details(EXTRACTION_MODEL_ID, usage.as_ref());
            audit_details["client_id"] = serde_json::json!(id.to_string());
            ctx.record_audit(
                ctx.audit_event("extract_document_text", "record_file", &filename)
                    .with_details(audit_details),
            )
            .await;

            text
        } else if let Some(media_format) =
            claria_transcribe::media_format_for_extension(&extension)
        {
            // Audio transcription using saved preferences (re-extract path).
            let options = build_transcribe_options(&ctx.cfg.transcription, None);
            let mut result = claria_transcribe::transcribe_audio_with_options(
                &ctx.sdk_config,
                &ctx.bucket,
                &key,
                media_format,
                &options,
            )
            .await?;
            maybe_translate(&ctx, &mut result, ctx.cfg.transcription.translate_to_english).await;
            let text = claria_transcribe::format_transcript_body(&result);

            claria_storage::objects::put_object(
                &ctx.s3,
                &ctx.bucket,
                &sidecar_key,
                text.clone().into_bytes(),
                Some("text/plain"),
            )
            .await?;

            text
        } else {
            let output = claria_storage::objects::get_object_bounded(
                &ctx.s3,
                &ctx.bucket,
                &key,
                claria_core::record_text::MAX_RECORD_TEXT_BYTES,
            )
            .await?;
            claria_core::record_text::decode_record_text(&output.body)
                .map(ToString::to_string)
                .ok_or_else(|| {
                    CommandError::Msg(format!(
                        "{filename} is not printable UTF-8 text and has no supported extraction format"
                    ))
                })?
        };

        tracing::info!(client_id = %id, "resolved text for record file");

        Ok(RecordContext { filename, text })
    })
    .await
}

/// Helper: load all record context for a client, converting to bedrock types.
pub(crate) async fn load_record_context(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    client_id: &str,
    cache: &claria_records::RecordCache,
) -> Result<Vec<claria_bedrock::context::ContextFile>, CommandError> {
    let id = parse_uuid(client_id)?;

    let texts = claria_records::fetch_record_texts(s3, bucket, id, cache).await?;

    Ok(texts
        .into_iter()
        .filter_map(|(filename, text)| {
            text.map(|text| claria_bedrock::context::ContextFile { filename, text })
        })
        .collect())
}
