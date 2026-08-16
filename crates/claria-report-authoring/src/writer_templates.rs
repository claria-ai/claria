//! Compatibility layer over the store's writer template library.
//!
//! The library itself is `claria_report_store::template_library`; this module
//! keeps the original paths, signatures, and error type working.

use aws_sdk_s3::Client as S3Client;
use uuid::Uuid;

use crate::ReportAuthoringError;

pub use claria_report_store::template_library::{
    MAX_WRITER_TEMPLATE_NAME_CHARACTERS, WriterTemplateMetadata, WriterTemplateSummary,
};

pub fn normalized_name(name: &str) -> Result<String, ReportAuthoringError> {
    claria_report_store::template_library::normalized_name(name).map_err(Into::into)
}

pub async fn create(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
    bytes: Vec<u8>,
) -> Result<WriterTemplateSummary, ReportAuthoringError> {
    claria_report_store::template_library::create(s3, bucket, id, name, bytes)
        .await
        .map_err(Into::into)
}

pub async fn list(
    s3: &S3Client,
    bucket: &str,
) -> Result<Vec<WriterTemplateSummary>, ReportAuthoringError> {
    claria_report_store::template_library::list(s3, bucket)
        .await
        .map_err(Into::into)
}

pub async fn rename(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
) -> Result<WriterTemplateSummary, ReportAuthoringError> {
    claria_report_store::template_library::rename(s3, bucket, id, name)
        .await
        .map_err(Into::into)
}

pub async fn delete(s3: &S3Client, bucket: &str, id: Uuid) -> Result<(), ReportAuthoringError> {
    claria_report_store::template_library::delete(s3, bucket, id)
        .await
        .map_err(Into::into)
}

pub async fn load_docx(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    max_bytes: u64,
) -> Result<Vec<u8>, ReportAuthoringError> {
    claria_report_store::template_library::load_docx(s3, bucket, id, max_bytes)
        .await
        .map_err(Into::into)
}

pub async fn load_docx_with_metadata(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    max_bytes: u64,
) -> Result<(WriterTemplateMetadata, Vec<u8>), ReportAuthoringError> {
    claria_report_store::template_library::load_docx_with_metadata(s3, bucket, id, max_bytes)
        .await
        .map_err(Into::into)
}

pub async fn increment_usage(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
) -> Result<u64, ReportAuthoringError> {
    claria_report_store::template_library::increment_usage(s3, bucket, id)
        .await
        .map_err(Into::into)
}
