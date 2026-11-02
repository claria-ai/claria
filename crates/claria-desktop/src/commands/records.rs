//! Record file commands — files attached to a client record.

use claria_core::processing::{ProcessingFailureCode, ProcessingStatus, usable_text};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{
    CommandContext, CommandError, parse_uuid,
    prompts::load_prompt,
    run,
    transcribe::{TranscribeOptionsOverrides, build_transcribe_options, maybe_translate},
    usage_audit_details,
};
use crate::state::DesktopState;
use claria_storage::audit::actions;

/// A file in a client's record (S3 object metadata).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RecordFile {
    pub filename: String,
    pub size: i32,
    pub uploaded_at: Option<String>,
    #[serde(default)]
    pub processing_status: Option<ProcessingStatus>,
}

/// Content type for a known upload extension. Unknown extensions fall back
/// to content validation in [`upload_record_file_inner`] — printable UTF-8
/// gets `text/plain`, anything else no content type.
fn content_type_for_extension(extension: &str) -> Option<&'static str> {
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

        let objects = claria_storage::objects::list_objects_with_metadata(
            &ctx.s3,
            &ctx.bucket,
            &records_prefix,
        )
        .await?;

        // Collect all keys into a set so we can check for base files when filtering sidecars.
        let all_keys: std::collections::HashSet<&str> =
            objects.iter().map(|o| o.key.as_str()).collect();

        let candidates: Vec<String> = objects
            .iter()
            .filter(|object| object.key.ends_with(".error"))
            .map(|object| object.key.clone())
            .collect();
        let errors = claria_storage::objects::recognized_objects(
            &ctx.s3,
            &ctx.bucket,
            &candidates,
            claria_core::processing::MAX_ERROR_BYTES,
            claria_core::processing::ProcessingError::parse,
        )
        .await?;
        let files: Vec<RecordFile> = objects
            .iter()
            .filter(|obj| obj.key.starts_with(&list_prefix) && !errors.contains_key(&obj.key))
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
                    processing_status: None,
                })
            })
            .collect();

        let files: Vec<RecordFile> = futures::stream::iter(files.into_iter().map(|mut file| {
            let ctx = &ctx;
            async move {
                if !file.filename.starts_with("chat-history/") {
                    file.processing_status =
                        Some(record_processing_status(ctx, id, &file.filename).await?);
                }
                Ok::<_, CommandError>(file)
            }
        }))
        .buffered(claria_storage::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()?;
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
/// Bedrock; audio files receive a transcript sidecar. Sidecar generation
/// failure degrades to a warning — the original upload has already succeeded.
#[tauri::command]
#[specta::specta]
pub async fn upload_record_file(
    state: State<'_, DesktopState>,
    client_id: String,
    file_path: String,
    expected_scope: Option<String>,
) -> Result<RecordFile, String> {
    run("upload_record_file", async {
        let ctx = CommandContext::new(&state).await?;
        check_processing_scope(&ctx, expected_scope.as_deref())?;
        // Drag-drop path: saved transcription preferences as-is.
        upload_record_file_inner(&ctx, &client_id, &file_path, None, false).await
    })
    .await
}

/// Upload an audio file and transcribe with the wizard's per-file options.
///
/// Same skeleton as [`upload_record_file`], but restricted to audio and with
/// a fatal sidecar: the wizard exists to produce a transcript, so a failed
/// transcription fails the command instead of degrading to a warning.
#[tauri::command]
#[specta::specta]
pub async fn upload_record_file_with_options(
    state: State<'_, DesktopState>,
    client_id: String,
    file_path: String,
    overrides: Option<TranscribeOptionsOverrides>,
) -> Result<RecordFile, String> {
    run("upload_record_file_with_options", async {
        let ctx = CommandContext::new(&state).await?;
        let extension = file_extension(std::path::Path::new(&file_path));
        if claria_transcribe::media_format_for_extension(&extension).is_none() {
            return Err(CommandError::Msg(format!(
                "Unsupported audio format: .{extension}"
            )));
        }
        upload_record_file_inner(&ctx, &client_id, &file_path, overrides, true).await
    })
    .await
}

fn file_extension(path: &std::path::Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// The one upload skeleton behind both upload commands: read, upload, audit,
/// then generate the appropriate `.text` sidecar.
///
/// `sidecar_failure_fatal` is the only behavioral fork: the wizard treats a
/// failed sidecar as a failed command, drag-drop degrades it to a warning.
// Spans and log lines carry the client UUID, extension, and byte count —
// never the client-chosen filename, which is PHI.
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(
        client_id = %client_id,
        extension = tracing::field::Empty,
        bytes = tracing::field::Empty,
        sidecar_failure_fatal,
    )
)]
async fn upload_record_file_inner(
    ctx: &CommandContext,
    client_id: &str,
    file_path: &str,
    overrides: Option<TranscribeOptionsOverrides>,
    sidecar_failure_fatal: bool,
) -> Result<RecordFile, CommandError> {
    let id = parse_uuid(client_id)?;

    let path = std::path::Path::new(file_path);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CommandError::Msg("Invalid file path".to_string()))?;

    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| CommandError::Msg(format!("Failed to read file: {e}")))?;
    let file_size = i32::try_from(metadata.len()).unwrap_or(i32::MAX);

    let extension = file_extension(path);

    let span = tracing::Span::current();
    span.record("extension", extension.as_str());
    span.record("bytes", metadata.len());

    // Only two cases need file bytes in memory: documents (Bedrock extraction
    // consumes them) and unknown extensions (printable-UTF-8 validation
    // decides the content type, bounded by the text-read ceiling). Everything
    // else streams straight from disk.
    let known_content_type = content_type_for_extension(&extension);
    let is_document = claria_bedrock::extract::document_format_for_extension(&extension).is_some();
    let mut document_bytes: Option<Vec<u8>> = None;
    let content_type = if is_document {
        document_bytes = Some(
            tokio::fs::read(path)
                .await
                .map_err(|e| CommandError::Msg(format!("Failed to read file: {e}")))?,
        );
        known_content_type
    } else if known_content_type.is_some() {
        known_content_type
    } else if metadata.len() <= claria_core::record_text::MAX_RECORD_TEXT_BYTES {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| CommandError::Msg(format!("Failed to read file: {e}")))?;
        claria_core::record_text::decode_record_text(&bytes).map(|_| "text/plain; charset=utf-8")
    } else {
        None
    };

    // Upload the original file, streaming its body from disk.
    let key = claria_core::s3_keys::client_record_file(id, filename);
    // Only binary processing needs the outcome path. Printable originals and
    // unsupported raw uploads must remain usable beside ordinary user files.
    let supports_processing =
        is_document || claria_transcribe::media_format_for_extension(&extension).is_some();
    if supports_processing
        && claria_records::processing::observe_error(&ctx.s3, &ctx.bucket, &key)
            .await?
            .conflicts_with_user_file()
    {
        let printable = if let Some(bytes) = document_bytes.as_deref() {
            usable_text(bytes).is_some()
        } else if metadata.len() <= claria_core::record_text::MAX_RECORD_TEXT_BYTES {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|e| CommandError::Msg(format!("Failed to read file: {e}")))?;
            usable_text(&bytes).is_some()
        } else {
            false
        };
        if !printable {
            return Err(CommandError::Msg(
                ProcessingFailureCode::ErrorFileConflict.reason().into(),
            ));
        }
    }
    claria_storage::objects::put_object_from_path(&ctx.s3, &ctx.bucket, &key, path, content_type)
        .await?;

    tracing::info!(client_id = %id, extension, "record file uploaded");

    // The durable audit trail may carry the filename; it lives only in
    // access-controlled S3.
    ctx.record_audit(
        ctx.audit_event(actions::RECORD_FILE_UPLOAD, "record_file", filename)
            .with_client_id(id)
            .with_details(serde_json::json!({"bytes": file_size,
            })),
    )
    .await;

    let processing_status = process_record(
        ctx,
        id,
        filename,
        &extension,
        &key,
        document_bytes.as_deref(),
        overrides,
    )
    .await?;
    if sidecar_failure_fatal && let ProcessingStatus::Failed { ref reason, .. } = processing_status
    {
        return Err(CommandError::Msg(format!("Original saved. {reason}")));
    }

    Ok(RecordFile {
        filename: filename.to_string(),
        size: file_size,
        uploaded_at: Some(jiff::Timestamp::now().to_string()),
        processing_status: Some(processing_status),
    })
}

/// Write the `.text` sidecar for a just-uploaded record file: structured
/// Markdown for documents, a rendered transcript for audio, nothing for
/// directly readable files.
async fn generate_upload_sidecar(
    ctx: &CommandContext,
    id: uuid::Uuid,
    filename: &str,
    extension: &str,
    key: &str,
    document_bytes: Option<&[u8]>,
    overrides: Option<TranscribeOptionsOverrides>,
) -> Result<(), CommandError> {
    let sidecar_key = format!("{key}.text");
    if document_bytes.is_some_and(|bytes| usable_text(bytes).is_some()) {
        return Ok(());
    }

    if let Some(format) = claria_bedrock::extract::document_format_for_extension(extension) {
        let bytes = document_bytes.ok_or_else(|| {
            CommandError::Msg("document bytes were not read before extraction".to_string())
        })?;
        let model_id = ctx
            .cfg
            .extraction_model_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| {
                CommandError::Msg(ProcessingFailureCode::ModelNotConfigured.reason().into())
            })?;
        let extraction_prompt = load_prompt(&ctx.s3, &ctx.bucket, "pdf-extraction").await?;
        let (text, usage) = claria_bedrock::extract::extract_document_text(
            &ctx.sdk_config,
            model_id,
            bytes,
            filename,
            format,
            &extraction_prompt,
        )
        .await?;

        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            &sidecar_key,
            validated_extraction(&text)?.as_bytes().to_vec(),
            Some("text/markdown; charset=utf-8"),
        )
        .await?;

        let mut audit_details = usage_audit_details(model_id, usage.as_ref(), None);
        audit_details["client_id"] = serde_json::json!(id.to_string());
        ctx.record_audit(
            ctx.audit_event(actions::RECORD_EXTRACT_TEXT, "record_file", filename)
                .with_details(audit_details),
        )
        .await;

        tracing::info!(client_id = %id, "sidecar text extraction uploaded");
    } else if let Some(media_format) = claria_transcribe::media_format_for_extension(extension) {
        let translate = overrides
            .as_ref()
            .and_then(|o| o.translate_to_english)
            .unwrap_or(ctx.cfg.transcription.translate_to_english);
        let options = build_transcribe_options(&ctx.cfg.transcription, overrides);

        let mut result = claria_transcribe::transcribe_audio_with_options(
            &ctx.sdk_config,
            &ctx.bucket,
            key,
            media_format,
            &options,
        )
        .await?;
        maybe_translate(ctx, &mut result, translate).await;
        let body = claria_transcribe::format_transcript_body(&result);
        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            &sidecar_key,
            validated_extraction(&body)?.as_bytes().to_vec(),
            Some("text/plain"),
        )
        .await?;

        tracing::info!(client_id = %id, "sidecar audio transcription uploaded");
    }

    Ok(())
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
            ctx.audit_event(actions::RECORD_TRANSCRIPT_EDIT, "transcript", &filename)
                .with_client_id(id),
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
            ctx.audit_event(actions::RECORD_FILE_DELETE, "record_file", &filename)
                .with_client_id(id),
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
            ctx.audit_event(actions::RECORD_FILE_CREATE, "record_file", &filename)
                .with_client_id(id)
                .with_details(serde_json::json!({"bytes": file_size,
                })),
        )
        .await;

        Ok(RecordFile {
            filename,
            size: file_size,
            uploaded_at: Some(jiff::Timestamp::now().to_string()),
            processing_status: Some(ProcessingStatus::Ready),
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
            ctx.audit_event(actions::RECORD_FILE_UPDATE, "record_file", &filename)
                .with_client_id(id),
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
    expected_scope: Option<String>,
) -> Result<RecordContext, String> {
    run("extract_record_file", async {
        let ctx = CommandContext::new(&state).await?;
        check_processing_scope(&ctx, expected_scope.as_deref())?;

        let id = parse_uuid(&client_id)?;
        let key = claria_core::s3_keys::client_record_file(id, &filename);

        let extension = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let bytes = if claria_bedrock::extract::document_format_for_extension(&extension).is_some()
        {
            Some(
                claria_storage::objects::get_object(&ctx.s3, &ctx.bucket, &key)
                    .await?
                    .body,
            )
        } else {
            match claria_storage::objects::get_object_bounded(
                &ctx.s3,
                &ctx.bucket,
                &key,
                claria_core::record_text::MAX_RECORD_TEXT_BYTES,
            )
            .await
            {
                Ok(output) => Some(output.body),
                Err(claria_storage::error::StorageError::ObjectTooLarge { .. }) => None,
                Err(error) => return Err(error.into()),
            }
        };
        let status = process_record(
            &ctx,
            id,
            &filename,
            &extension,
            &key,
            bytes.as_deref(),
            None,
        )
        .await?;
        if let ProcessingStatus::Failed { reason, .. } = status {
            return Err(CommandError::Msg(reason));
        }
        let text = claria_records::fetch_record_text(
            &ctx.s3,
            &ctx.bucket,
            id,
            &filename,
            &state.record_cache,
        )
        .await?
        .ok_or_else(|| {
            CommandError::Msg(
                "No usable text is available. A newer processing failure may need another retry."
                    .into(),
            )
        })?;

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

fn validated_extraction(text: &str) -> Result<&str, CommandError> {
    usable_text(text.as_bytes())
        .filter(|_| text.len() as u64 <= claria_core::record_text::MAX_RECORD_TEXT_BYTES)
        .ok_or_else(|| CommandError::Msg(ProcessingFailureCode::InvalidText.reason().into()))
}

async fn record_processing_status(
    ctx: &CommandContext,
    id: uuid::Uuid,
    filename: &str,
) -> Result<ProcessingStatus, CommandError> {
    let key = claria_core::s3_keys::client_record_file(id, filename);
    match claria_storage::objects::get_object_bounded(
        &ctx.s3,
        &ctx.bucket,
        &key,
        claria_core::record_text::MAX_RECORD_TEXT_BYTES,
    )
    .await
    {
        Ok(output) if usable_text(&output.body).is_some() => return Ok(ProcessingStatus::Ready),
        Ok(_) | Err(claria_storage::error::StorageError::ObjectTooLarge { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let observed = claria_records::processing::observe_error(&ctx.s3, &ctx.bucket, &key).await?;
    if let Some(error) = observed.marker {
        return Ok(ProcessingStatus::Failed {
            code: error.code,
            reason: error.reason,
        });
    }
    let sidecar = claria_storage::objects::get_object_bounded(
        &ctx.s3,
        &ctx.bucket,
        &format!("{key}.text"),
        claria_core::record_text::MAX_RECORD_TEXT_BYTES,
    )
    .await;
    match sidecar {
        Ok(output) if usable_text(&output.body).is_some() => return Ok(ProcessingStatus::Ready),
        Ok(_)
        | Err(
            claria_storage::error::StorageError::NotFound { .. }
            | claria_storage::error::StorageError::ObjectTooLarge { .. },
        ) => {}
        Err(error) => return Err(error.into()),
    }
    let extension = file_extension(std::path::Path::new(filename));
    Ok(
        if claria_bedrock::extract::document_format_for_extension(&extension).is_some()
            || claria_transcribe::media_format_for_extension(&extension).is_some()
        {
            ProcessingStatus::Unknown
        } else {
            ProcessingStatus::NotSupported
        },
    )
}

async fn process_record(
    ctx: &CommandContext,
    id: uuid::Uuid,
    filename: &str,
    extension: &str,
    key: &str,
    bytes: Option<&[u8]>,
    overrides: Option<TranscribeOptionsOverrides>,
) -> Result<ProcessingStatus, CommandError> {
    let printable = if let Some(bytes) = bytes {
        usable_text(bytes).is_some()
    } else {
        match claria_storage::objects::get_object_bounded(
            &ctx.s3,
            &ctx.bucket,
            key,
            claria_core::record_text::MAX_RECORD_TEXT_BYTES,
        )
        .await
        {
            Ok(output) => usable_text(&output.body).is_some(),
            Err(claria_storage::error::StorageError::ObjectTooLarge { .. }) => false,
            Err(error) => return Err(error.into()),
        }
    };
    if printable {
        return Ok(ProcessingStatus::Ready);
    }
    let supported = claria_bedrock::extract::document_format_for_extension(extension).is_some()
        || claria_transcribe::media_format_for_extension(extension).is_some();
    if !supported {
        return record_processing_status(ctx, id, filename).await;
    }
    let observed = claria_records::processing::observe_error(&ctx.s3, &ctx.bucket, key).await?;
    if observed.conflicts_with_user_file() {
        let code = ProcessingFailureCode::ErrorFileConflict;
        return Ok(ProcessingStatus::Failed {
            code,
            reason: code.reason().into(),
        });
    }
    let result = generate_upload_sidecar(ctx, id, filename, extension, key, bytes, overrides).await;
    match result {
        Ok(()) => {
            claria_records::processing::clear_observed_error(&ctx.s3, &ctx.bucket, key, &observed)
                .await?;
            record_processing_status(ctx, id, filename).await
        }
        Err(error) => {
            let code = match &error {
                CommandError::Bedrock(claria_bedrock::error::BedrockError::ResponseTruncated {
                    ..
                }) => ProcessingFailureCode::Truncated,
                CommandError::Bedrock(claria_bedrock::error::BedrockError::Service {
                    code,
                    ..
                }) if code == "AccessDeniedException" => ProcessingFailureCode::AccessDenied,
                CommandError::Bedrock(claria_bedrock::error::BedrockError::Service {
                    code,
                    ..
                }) if code == "ResourceNotFoundException" || code == "ValidationException" => {
                    ProcessingFailureCode::ModelUnavailable
                }
                CommandError::Bedrock(claria_bedrock::error::BedrockError::Service {
                    code,
                    ..
                }) if code == "ThrottlingException" || code == "ServiceQuotaExceededException" => {
                    ProcessingFailureCode::Throttled
                }
                CommandError::Storage(_) => ProcessingFailureCode::StorageUnavailable,
                CommandError::Msg(message)
                    if message == ProcessingFailureCode::ModelNotConfigured.reason() =>
                {
                    ProcessingFailureCode::ModelNotConfigured
                }
                CommandError::Msg(message)
                    if message == ProcessingFailureCode::InvalidText.reason() =>
                {
                    ProcessingFailureCode::InvalidText
                }
                _ => ProcessingFailureCode::ProviderUnavailable,
            };
            // Persist only canonical prose, never the provider diagnostic.
            let saved =
                claria_records::processing::save_error(&ctx.s3, &ctx.bucket, key, &observed, code)
                    .await;
            let reason = match saved {
                Ok(true) => code.reason().to_string(),
                Ok(false) => format!("{} The existing error file was preserved.", code.reason()),
                Err(_) => format!(
                    "{} The failure could not be saved; keep this message and retry after checking storage access.",
                    code.reason()
                ),
            };
            tracing::warn!(client_id = %id, ?code, "record processing failed; original retained");
            Ok(ProcessingStatus::Failed { code, reason })
        }
    }
}

fn check_processing_scope(
    ctx: &CommandContext,
    expected: Option<&str>,
) -> Result<(), CommandError> {
    let actual = format!(
        "{}:{}:{}",
        ctx.cfg.account_id, ctx.cfg.region, ctx.cfg.system_name
    );
    if expected.is_some_and(|expected| expected != actual) {
        return Err(CommandError::Msg(
            "The active account changed. Return to the record before processing files.".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/commands/record_processing.rs"]
mod processing_tests;
