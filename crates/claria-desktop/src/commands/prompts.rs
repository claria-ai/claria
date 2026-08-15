//! Editable prompts stored under `claria-prompts/` in S3, plus their
//! version history.

use claria_report_authoring::writer_prompts::WriterPrompt;
use tauri::State;

use super::{
    CommandContext, CommandError, parse_uuid, run,
    versions::{
        FileVersion, get_version_text_for_key, list_versions_for_key, restore_version_for_key,
    },
};
use crate::state::DesktopState;

/// Default system prompt, used when no custom prompt has been saved to S3.
const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a clinical assistant helping a psychologist set up a new client record. \
Help gather relevant intake information such as the client's presenting concerns, \
referral source, relevant history, and initial observations. \
Be professional, empathetic, and concise. Ask clarifying questions when needed. \
Do not provide diagnoses or treatment recommendations — your role is to help \
organize and document the intake information.";

/// Resolve a prompt name to its S3 key and hardcoded default text.
///
/// Returns `(s3_key, legacy_key, default_text)`. The `legacy_key` is `Some`
/// only for the system prompt which was previously stored at the bucket root.
fn resolve_prompt(
    name: &str,
) -> Result<(&'static str, Option<&'static str>, &'static str), CommandError> {
    match name {
        "system-prompt" => Ok((
            claria_core::s3_keys::SYSTEM_PROMPT,
            Some(claria_core::s3_keys::LEGACY_SYSTEM_PROMPT),
            DEFAULT_SYSTEM_PROMPT,
        )),
        "pdf-extraction" => Ok((
            claria_core::s3_keys::EXTRACTION_PROMPT,
            None,
            claria_bedrock::extract::DEFAULT_EXTRACTION_PROMPT,
        )),
        // Only the editable body is stored; the fixed trust rules are always
        // appended by claria-report-authoring's prompt composition.
        "report-system" => Ok((
            claria_core::s3_keys::REPORT_SYSTEM_PROMPT,
            None,
            claria_report_authoring::REPORT_SYSTEM_PROMPT_BODY,
        )),
        "report-full-draft" => Ok((
            claria_core::s3_keys::FULL_REPORT_SYSTEM_PROMPT,
            None,
            claria_report_authoring::FULL_REPORT_SYSTEM_PROMPT_BODY,
        )),
        _ => Err(CommandError::Msg(format!("unknown prompt name: {name}"))),
    }
}

/// The fixed trust-boundary rules Claria appends to every writer prompt —
/// surfaced so Preferences can display exactly what always runs, without
/// making it editable.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct WriterTrustRules {
    pub targeted: String,
    pub full_draft: String,
}

#[tauri::command]
#[specta::specta]
pub async fn get_writer_trust_rules() -> Result<WriterTrustRules, String> {
    Ok(WriterTrustRules {
        targeted: claria_report_authoring::REPORT_TRUST_RULES.to_string(),
        full_draft: claria_report_authoring::FULL_REPORT_TRUST_RULES.to_string(),
    })
}

/// Load a prompt from S3 by name, falling back to the legacy path and then the
/// hardcoded default.
pub(crate) async fn load_prompt(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    prompt_name: &str,
) -> Result<String, CommandError> {
    let (key, legacy_key, default_text) = resolve_prompt(prompt_name)?;

    // Try the canonical claria-prompts/ key first.
    match claria_storage::objects::get_object(s3, bucket, key).await {
        Ok(output) => {
            return String::from_utf8(output.body).map_err(|e| CommandError::Msg(e.to_string()));
        }
        Err(claria_storage::error::StorageError::NotFound { .. }) => {}
        Err(e) => return Err(e.into()),
    }

    // Fall back to the legacy key if one exists (system-prompt.md at bucket root).
    // When found, migrate it to the new path and delete the legacy key.
    if let Some(legacy) = legacy_key {
        match claria_storage::objects::get_object(s3, bucket, legacy).await {
            Ok(output) => {
                let text =
                    String::from_utf8(output.body).map_err(|e| CommandError::Msg(e.to_string()))?;

                // Copy to the new claria-prompts/ path.
                if let Err(e) = claria_storage::objects::put_object(
                    s3,
                    bucket,
                    key,
                    text.as_bytes().to_vec(),
                    Some("text/markdown"),
                )
                .await
                {
                    tracing::warn!(legacy, key, error = %e, "failed to migrate legacy prompt");
                    return Ok(text);
                }

                // Remove the legacy key.
                if let Err(e) = claria_storage::objects::delete_object(s3, bucket, legacy).await {
                    tracing::warn!(legacy, error = %e, "failed to delete legacy prompt after migration");
                }

                tracing::info!(legacy, key, "migrated legacy prompt to claria-prompts/");
                return Ok(text);
            }
            Err(claria_storage::error::StorageError::NotFound { .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }

    Ok(default_text.to_string())
}

/// Get the current content of a named prompt.
///
/// Returns the custom prompt from S3 if one exists, otherwise returns the
/// built-in default. Valid prompt names: `"system-prompt"`, `"pdf-extraction"`.
#[tauri::command]
#[specta::specta]
pub async fn get_prompt(
    state: State<'_, DesktopState>,
    prompt_name: String,
) -> Result<String, String> {
    run("get_prompt", async {
        let ctx = CommandContext::new(&state).await?;

        load_prompt(&ctx.s3, &ctx.bucket, &prompt_name).await
    })
    .await
}

/// Save a named prompt to S3.
///
/// Overwrites any previously saved version. The new content takes effect on
/// the next operation that uses this prompt.
#[tauri::command]
#[specta::specta]
pub async fn save_prompt(
    state: State<'_, DesktopState>,
    prompt_name: String,
    content: String,
) -> Result<(), String> {
    run("save_prompt", async {
        let (key, _, _) = resolve_prompt(&prompt_name)?;

        let ctx = CommandContext::new(&state).await?;

        claria_storage::objects::put_object(
            &ctx.s3,
            &ctx.bucket,
            key,
            content.into_bytes(),
            Some("text/markdown"),
        )
        .await?;

        ctx.record_audit(ctx.audit_event("prompt_saved", "prompt", &prompt_name))
            .await;

        Ok(())
    })
    .await
}

/// Delete a named prompt from S3, reverting to the built-in default.
#[tauri::command]
#[specta::specta]
pub async fn delete_prompt(
    state: State<'_, DesktopState>,
    prompt_name: String,
) -> Result<(), String> {
    run("delete_prompt", async {
        let (key, _, _) = resolve_prompt(&prompt_name)?;

        let ctx = CommandContext::new(&state).await?;

        claria_storage::objects::delete_object(&ctx.s3, &ctx.bucket, key).await?;

        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Writer prompt library commands
// ---------------------------------------------------------------------------

/// List the saved writer prompts, alphabetically by name.
#[tauri::command]
#[specta::specta]
pub async fn list_writer_library_prompts(
    state: State<'_, DesktopState>,
) -> Result<Vec<WriterPrompt>, String> {
    run("list_writer_library_prompts", async {
        let ctx = CommandContext::new(&state).await?;
        Ok(claria_report_authoring::writer_prompts::list(&ctx.s3, &ctx.bucket).await?)
    })
    .await
}

/// Create (no `prompt_id`) or update (with `prompt_id`) one saved writer
/// prompt.
#[tauri::command]
#[specta::specta]
pub async fn save_writer_library_prompt(
    state: State<'_, DesktopState>,
    prompt_id: Option<String>,
    name: String,
    body: String,
) -> Result<WriterPrompt, String> {
    run("save_writer_library_prompt", async {
        let ctx = CommandContext::new(&state).await?;
        let (prompt, action) = match prompt_id {
            None => (
                claria_report_authoring::writer_prompts::create(
                    &ctx.s3,
                    &ctx.bucket,
                    uuid::Uuid::new_v4(),
                    &name,
                    &body,
                )
                .await?,
                "writer_prompt_created",
            ),
            Some(prompt_id) => (
                claria_report_authoring::writer_prompts::update(
                    &ctx.s3,
                    &ctx.bucket,
                    parse_uuid(&prompt_id)?,
                    &name,
                    &body,
                )
                .await?,
                "writer_prompt_updated",
            ),
        };
        ctx.record_audit(ctx.audit_event(action, "writer_prompt", prompt.id.to_string()))
            .await;
        Ok(prompt)
    })
    .await
}

/// Delete one saved writer prompt.
#[tauri::command]
#[specta::specta]
pub async fn delete_writer_library_prompt(
    state: State<'_, DesktopState>,
    prompt_id: String,
) -> Result<(), String> {
    run("delete_writer_library_prompt", async {
        let prompt_id = parse_uuid(&prompt_id)?;
        let ctx = CommandContext::new(&state).await?;
        claria_report_authoring::writer_prompts::delete(&ctx.s3, &ctx.bucket, prompt_id).await?;
        ctx.record_audit(ctx.audit_event(
            "writer_prompt_deleted",
            "writer_prompt",
            prompt_id.to_string(),
        ))
        .await;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Prompt version history commands
// ---------------------------------------------------------------------------

/// List all versions of a named prompt stored in S3.
#[tauri::command]
#[specta::specta]
pub async fn list_prompt_versions(
    state: State<'_, DesktopState>,
    prompt_name: String,
) -> Result<Vec<FileVersion>, String> {
    run("list_prompt_versions", async {
        let (key, _, _) = resolve_prompt(&prompt_name)?;

        let ctx = CommandContext::new(&state).await?;

        list_versions_for_key(&ctx.s3, &ctx.bucket, key).await
    })
    .await
}

/// Get the text content of a specific version of a named prompt.
#[tauri::command]
#[specta::specta]
pub async fn get_prompt_version(
    state: State<'_, DesktopState>,
    prompt_name: String,
    version_id: String,
) -> Result<String, String> {
    run("get_prompt_version", async {
        let (key, _, _) = resolve_prompt(&prompt_name)?;

        let ctx = CommandContext::new(&state).await?;

        get_version_text_for_key(&ctx.s3, &ctx.bucket, key, &version_id).await
    })
    .await
}

/// Restore a previous version of a named prompt by writing it as the new current version.
#[tauri::command]
#[specta::specta]
pub async fn restore_prompt_version(
    state: State<'_, DesktopState>,
    prompt_name: String,
    version_id: String,
) -> Result<(), String> {
    run("restore_prompt_version", async {
        let (key, _, _) = resolve_prompt(&prompt_name)?;

        let ctx = CommandContext::new(&state).await?;

        restore_version_for_key(
            &ctx.s3,
            &ctx.bucket,
            key,
            &version_id,
            Some("text/markdown"),
        )
        .await?;

        tracing::info!(prompt_name, version_id, "prompt version restored");

        ctx.record_audit(
            ctx.audit_event("prompt_version_restored", "prompt", &prompt_name)
                .with_details(serde_json::json!({ "version_id": version_id })),
        )
        .await;

        Ok(())
    })
    .await
}
