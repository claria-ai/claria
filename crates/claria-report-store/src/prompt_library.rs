//! Small S3-backed library of reusable writer steering prompts.
//!
//! A saved prompt is a named instruction the user picks to prefill the
//! writer's instruction box — whole-report guidance ("Fill referral and
//! background; skip the rest") or a targeted request ("Draft the summary
//! backing my diagnosis of …"). The picked body is ordinary editable input,
//! never a system-prompt fragment, so the turn loop and trust rules are
//! untouched.

use aws_sdk_s3::Client as S3Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ReportStoreError;

const PROMPT_SCHEMA_VERSION: u32 = 1;
pub const MAX_WRITER_PROMPT_NAME_CHARACTERS: usize = 120;
/// Bodies prefill the instruction box, so their ceiling is the instruction
/// ceiling — a saved prompt must always be submittable unedited.
pub const MAX_WRITER_PROMPT_BODY_CHARACTERS: usize = crate::MAX_INSTRUCTION_CHARACTERS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct WriterPrompt {
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    pub body: String,
    #[specta(type = String)]
    pub created_at: jiff::Timestamp,
    #[specta(type = String)]
    pub updated_at: jiff::Timestamp,
}

fn normalized_name(name: &str) -> Result<String, ReportStoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ReportStoreError::InvalidInput(
            "Enter a name for the saved prompt.".to_string(),
        ));
    }
    if name.chars().count() > MAX_WRITER_PROMPT_NAME_CHARACTERS {
        return Err(ReportStoreError::InvalidInput(format!(
            "Saved prompt names may contain at most {MAX_WRITER_PROMPT_NAME_CHARACTERS} characters."
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(ReportStoreError::InvalidInput(
            "Saved prompt names cannot contain control characters.".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn normalized_body(body: &str) -> Result<String, ReportStoreError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(ReportStoreError::InvalidInput(
            "Enter the prompt text to save.".to_string(),
        ));
    }
    if body.chars().count() > MAX_WRITER_PROMPT_BODY_CHARACTERS {
        return Err(ReportStoreError::InvalidInput(format!(
            "Saved prompts may contain at most {MAX_WRITER_PROMPT_BODY_CHARACTERS} characters."
        )));
    }
    Ok(body.to_string())
}

pub async fn create(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
    body: &str,
) -> Result<WriterPrompt, ReportStoreError> {
    if id.is_nil() {
        return Err(ReportStoreError::InvalidInput(
            "The saved prompt ID cannot be nil.".to_string(),
        ));
    }
    let now = jiff::Timestamp::now();
    let prompt = WriterPrompt {
        schema_version: PROMPT_SCHEMA_VERSION,
        id,
        name: normalized_name(name)?,
        body: normalized_body(body)?,
        created_at: now,
        updated_at: now,
    };
    save(s3, bucket, &prompt).await?;
    Ok(prompt)
}

pub async fn list(s3: &S3Client, bucket: &str) -> Result<Vec<WriterPrompt>, ReportStoreError> {
    let keys = claria_storage::objects::list_objects(
        s3,
        bucket,
        claria_core::s3_keys::WRITER_PROMPT_LIBRARY_PREFIX,
    )
    .await
    .map_err(|source| ReportStoreError::storage("listing saved prompts", source))?;
    let mut prompts = Vec::new();
    for key in keys {
        if !key.ends_with(".json") {
            continue;
        }
        let output = claria_storage::objects::get_object(s3, bucket, &key)
            .await
            .map_err(|source| ReportStoreError::storage("reading a saved prompt", source))?;
        let prompt: WriterPrompt = serde_json::from_slice(&output.body).map_err(|_| {
            ReportStoreError::InvalidInput("A saved prompt is invalid.".to_string())
        })?;
        if prompt.schema_version != PROMPT_SCHEMA_VERSION
            || prompt.id.is_nil()
            || key != claria_core::s3_keys::writer_library_prompt(prompt.id)
        {
            return Err(ReportStoreError::InvalidInput(
                "A saved prompt does not match its stored key.".to_string(),
            ));
        }
        prompts.push(prompt);
    }
    // Alphabetical for a stable picker; recency is a poor recall key for a
    // library the user reads by name.
    prompts.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    Ok(prompts)
}

pub async fn update(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
    body: &str,
) -> Result<WriterPrompt, ReportStoreError> {
    let mut prompt = load(s3, bucket, id).await?;
    prompt.name = normalized_name(name)?;
    prompt.body = normalized_body(body)?;
    prompt.updated_at = jiff::Timestamp::now();
    save(s3, bucket, &prompt).await?;
    Ok(prompt)
}

pub async fn delete(s3: &S3Client, bucket: &str, id: Uuid) -> Result<(), ReportStoreError> {
    claria_storage::objects::delete_object(
        s3,
        bucket,
        &claria_core::s3_keys::writer_library_prompt(id),
    )
    .await
    .map_err(|source| ReportStoreError::storage("deleting a saved prompt", source))?;
    Ok(())
}

async fn load(s3: &S3Client, bucket: &str, id: Uuid) -> Result<WriterPrompt, ReportStoreError> {
    let (prompt, _) = claria_storage::state::load_state_checked(
        s3,
        bucket,
        &claria_core::s3_keys::writer_library_prompt(id),
        None,
        |prompt: &WriterPrompt| {
            if prompt.id != id || prompt.schema_version != PROMPT_SCHEMA_VERSION {
                return Err("The saved prompt does not match its stored key.".to_string());
            }
            Ok(())
        },
    )
    .await
    .map_err(|source| match source {
        claria_storage::error::StorageError::Serialization(_) => {
            ReportStoreError::InvalidInput("A saved prompt is invalid.".to_string())
        }
        claria_storage::error::StorageError::InvalidState { reason, .. } => {
            ReportStoreError::InvalidInput(reason)
        }
        other => ReportStoreError::storage("reading a saved prompt", other),
    })?;
    Ok(prompt)
}

async fn save(s3: &S3Client, bucket: &str, prompt: &WriterPrompt) -> Result<(), ReportStoreError> {
    let body = serde_json::to_vec_pretty(prompt).map_err(|_| {
        ReportStoreError::InvalidInput("Claria could not encode the saved prompt.".to_string())
    })?;
    claria_storage::objects::put_object(
        s3,
        bucket,
        &claria_core::s3_keys::writer_library_prompt(prompt.id),
        body,
        Some("application/json"),
    )
    .await
    .map_err(|source| ReportStoreError::storage("saving the prompt", source))?;
    Ok(())
}
