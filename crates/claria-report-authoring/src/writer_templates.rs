//! Small S3-backed library of reusable, redacted DOCX writer templates.

use aws_sdk_s3::Client as S3Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ReportAuthoringError;

const METADATA_SCHEMA_VERSION: u32 = 1;
const MAX_WRITER_TEMPLATE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_WRITER_TEMPLATE_NAME_CHARACTERS: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterTemplateMetadata {
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    pub size: u64,
    pub uploaded_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterTemplateSummary {
    pub metadata: WriterTemplateMetadata,
    pub use_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WriterTemplateUsage {
    #[serde(default)]
    use_count: u64,
}

pub fn normalized_name(name: &str) -> Result<String, ReportAuthoringError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ReportAuthoringError::InvalidInput(
            "Enter a writer template name.".to_string(),
        ));
    }
    if name.chars().count() > MAX_WRITER_TEMPLATE_NAME_CHARACTERS {
        return Err(ReportAuthoringError::InvalidInput(format!(
            "Writer template names may contain at most {MAX_WRITER_TEMPLATE_NAME_CHARACTERS} characters."
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(ReportAuthoringError::InvalidInput(
            "Writer template names cannot contain control characters.".to_string(),
        ));
    }
    Ok(name.to_string())
}

pub async fn create(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
    bytes: Vec<u8>,
) -> Result<WriterTemplateSummary, ReportAuthoringError> {
    let name = normalized_name(name)?;
    if bytes.is_empty() || bytes.len() > MAX_WRITER_TEMPLATE_BYTES {
        return Err(ReportAuthoringError::InvalidInput(
            "Writer templates must be between 1 byte and 10 MiB.".to_string(),
        ));
    }
    let size = u64::try_from(bytes.len()).map_err(|_| {
        ReportAuthoringError::InvalidInput("The writer template is too large.".to_string())
    })?;
    let metadata = WriterTemplateMetadata {
        schema_version: METADATA_SCHEMA_VERSION,
        id,
        name,
        size,
        uploaded_at: jiff::Timestamp::now(),
    };
    claria_storage::objects::put_object(
        s3,
        bucket,
        &claria_core::s3_keys::writer_template_docx(id),
        bytes,
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    )
    .await
    .map_err(|source| ReportAuthoringError::storage("uploading the writer template", source))?;
    save_metadata(s3, bucket, &metadata).await?;
    // Usage is informational. A counter failure must not turn a successful
    // template upload into a failed operation; reads will display zero.
    let _ = save_usage(s3, bucket, id, 0).await;
    Ok(WriterTemplateSummary {
        metadata,
        use_count: 0,
    })
}

pub async fn list(
    s3: &S3Client,
    bucket: &str,
) -> Result<Vec<WriterTemplateSummary>, ReportAuthoringError> {
    let keys = claria_storage::objects::list_objects(
        s3,
        bucket,
        claria_core::s3_keys::WRITER_TEMPLATES_PREFIX,
    )
    .await
    .map_err(|source| ReportAuthoringError::storage("listing writer templates", source))?;
    let mut templates = Vec::new();
    for key in keys {
        if !key.ends_with(".json") || key.ends_with(".usage.json") {
            continue;
        }
        let output = claria_storage::objects::get_object(s3, bucket, &key)
            .await
            .map_err(|source| {
                ReportAuthoringError::storage("reading writer template metadata", source)
            })?;
        let metadata: WriterTemplateMetadata =
            serde_json::from_slice(&output.body).map_err(|_| {
                ReportAuthoringError::InvalidInput(
                    "A stored writer template has invalid metadata.".to_string(),
                )
            })?;
        if metadata.schema_version != METADATA_SCHEMA_VERSION
            || metadata.id.is_nil()
            || key != claria_core::s3_keys::writer_template_metadata(metadata.id)
        {
            return Err(ReportAuthoringError::InvalidInput(
                "A stored writer template has unsupported metadata.".to_string(),
            ));
        }
        let use_count = read_usage(s3, bucket, metadata.id).await;
        templates.push(WriterTemplateSummary {
            metadata,
            use_count,
        });
    }
    templates.sort_by_key(|template| std::cmp::Reverse(template.metadata.uploaded_at));
    Ok(templates)
}

pub async fn rename(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
) -> Result<WriterTemplateSummary, ReportAuthoringError> {
    let mut metadata = load_metadata(s3, bucket, id).await?;
    metadata.name = normalized_name(name)?;
    save_metadata(s3, bucket, &metadata).await?;
    let use_count = read_usage(s3, bucket, id).await;
    Ok(WriterTemplateSummary {
        metadata,
        use_count,
    })
}

pub async fn delete(s3: &S3Client, bucket: &str, id: Uuid) -> Result<(), ReportAuthoringError> {
    // Remove metadata first so a partial failure cannot leave a visible row
    // whose source DOCX has already disappeared.
    for key in [
        claria_core::s3_keys::writer_template_metadata(id),
        claria_core::s3_keys::writer_template_usage(id),
        claria_core::s3_keys::writer_template_docx(id),
    ] {
        claria_storage::objects::delete_object(s3, bucket, &key)
            .await
            .map_err(|source| {
                ReportAuthoringError::storage("deleting a writer template", source)
            })?;
    }
    Ok(())
}

pub async fn load_docx(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    max_bytes: u64,
) -> Result<Vec<u8>, ReportAuthoringError> {
    // Require metadata too, so a stale/orphaned DOCX cannot be selected.
    load_metadata(s3, bucket, id).await?;
    let output = claria_storage::objects::get_object_bounded(
        s3,
        bucket,
        &claria_core::s3_keys::writer_template_docx(id),
        max_bytes,
    )
    .await
    .map_err(|source| ReportAuthoringError::storage("reading the writer template", source))?;
    Ok(output.body)
}

pub async fn increment_usage(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
) -> Result<u64, ReportAuthoringError> {
    // Missing or malformed counters deliberately restart at zero. Usage is a
    // convenience metric, not report state, and does not warrant locking.
    let next = read_usage(s3, bucket, id).await.saturating_add(1);
    save_usage(s3, bucket, id, next).await?;
    Ok(next)
}

async fn load_metadata(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
) -> Result<WriterTemplateMetadata, ReportAuthoringError> {
    let output = claria_storage::objects::get_object(
        s3,
        bucket,
        &claria_core::s3_keys::writer_template_metadata(id),
    )
    .await
    .map_err(|source| ReportAuthoringError::storage("reading writer template metadata", source))?;
    let metadata: WriterTemplateMetadata = serde_json::from_slice(&output.body).map_err(|_| {
        ReportAuthoringError::InvalidInput(
            "The stored writer template has invalid metadata.".to_string(),
        )
    })?;
    if metadata.id != id || metadata.schema_version != METADATA_SCHEMA_VERSION {
        return Err(ReportAuthoringError::InvalidInput(
            "The stored writer template metadata does not match its key.".to_string(),
        ));
    }
    Ok(metadata)
}

async fn save_metadata(
    s3: &S3Client,
    bucket: &str,
    metadata: &WriterTemplateMetadata,
) -> Result<(), ReportAuthoringError> {
    let body = serde_json::to_vec_pretty(metadata).map_err(|_| {
        ReportAuthoringError::InvalidInput(
            "Claria could not encode writer template metadata.".to_string(),
        )
    })?;
    claria_storage::objects::put_object(
        s3,
        bucket,
        &claria_core::s3_keys::writer_template_metadata(metadata.id),
        body,
        Some("application/json"),
    )
    .await
    .map_err(|source| ReportAuthoringError::storage("saving writer template metadata", source))?;
    Ok(())
}

async fn read_usage(s3: &S3Client, bucket: &str, id: Uuid) -> u64 {
    let result = claria_storage::objects::get_object(
        s3,
        bucket,
        &claria_core::s3_keys::writer_template_usage(id),
    )
    .await;
    match result {
        Ok(output) => serde_json::from_slice::<WriterTemplateUsage>(&output.body)
            .map(|usage| usage.use_count)
            .unwrap_or(0),
        Err(_) => 0,
    }
}

async fn save_usage(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    use_count: u64,
) -> Result<(), ReportAuthoringError> {
    let body = serde_json::to_vec_pretty(&WriterTemplateUsage { use_count }).map_err(|_| {
        ReportAuthoringError::InvalidInput(
            "Claria could not encode writer template usage.".to_string(),
        )
    })?;
    claria_storage::objects::put_object(
        s3,
        bucket,
        &claria_core::s3_keys::writer_template_usage(id),
        body,
        Some("application/json"),
    )
    .await
    .map_err(|source| ReportAuthoringError::storage("saving writer template usage", source))?;
    Ok(())
}
