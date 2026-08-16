//! Compatibility layer over the store's writer prompt library.
//!
//! The library itself is `claria_report_store::prompt_library`; this module
//! keeps the original paths, signatures, and error type working.

use aws_sdk_s3::Client as S3Client;
use uuid::Uuid;

use crate::ReportAuthoringError;

pub use claria_report_store::prompt_library::{
    MAX_WRITER_PROMPT_BODY_CHARACTERS, MAX_WRITER_PROMPT_NAME_CHARACTERS, WriterPrompt,
};

pub async fn create(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
    body: &str,
) -> Result<WriterPrompt, ReportAuthoringError> {
    claria_report_store::prompt_library::create(s3, bucket, id, name, body)
        .await
        .map_err(Into::into)
}

pub async fn list(s3: &S3Client, bucket: &str) -> Result<Vec<WriterPrompt>, ReportAuthoringError> {
    claria_report_store::prompt_library::list(s3, bucket)
        .await
        .map_err(Into::into)
}

pub async fn update(
    s3: &S3Client,
    bucket: &str,
    id: Uuid,
    name: &str,
    body: &str,
) -> Result<WriterPrompt, ReportAuthoringError> {
    claria_report_store::prompt_library::update(s3, bucket, id, name, body)
        .await
        .map_err(Into::into)
}

pub async fn delete(s3: &S3Client, bucket: &str, id: Uuid) -> Result<(), ReportAuthoringError> {
    claria_report_store::prompt_library::delete(s3, bucket, id)
        .await
        .map_err(Into::into)
}
