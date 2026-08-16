//! Compatibility layer over `claria-report-store`.
//!
//! Durable writer state now lives in its own crate, but the public writer API
//! is unchanged: every function below keeps its original signature and error
//! type and forwards to the store. Callers migrate to `claria_report_store`
//! directly when the writer crate is renamed; until then nothing outside this
//! crate has to move.

use aws_sdk_s3::Client as S3Client;
use claria_core::models::report::{
    ReportContent, ReportDraft, ReportExportStatus, ReportProposalDecision, ReportWorkspace,
};
use claria_report_store::RevisionCache;
use uuid::Uuid;

use crate::{
    ReportAuthoringError, ReportExportSnapshot, ReportRevisionSummary, ReportTemplateApplication,
};

pub async fn load_report_workspace(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::load_report_workspace(s3, bucket, client_id)
        .await
        .map_err(Into::into)
}

pub async fn start_report_workspace(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::start_report_workspace(s3, bucket, client_id)
        .await
        .map_err(Into::into)
}

pub async fn start_report_workspace_with_id(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::start_report_workspace_with_id(s3, bucket, client_id, report_id)
        .await
        .map_err(Into::into)
}

pub async fn load_report_workspace_by_id(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::load_report_workspace_by_id(s3, bucket, client_id, report_id)
        .await
        .map_err(Into::into)
}

pub async fn list_report_workspaces(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<Vec<ReportWorkspace>, ReportAuthoringError> {
    claria_report_store::list_report_workspaces(s3, bucket, client_id)
        .await
        .map_err(Into::into)
}

pub async fn find_report_workspace(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<Option<ReportWorkspace>, ReportAuthoringError> {
    claria_report_store::find_report_workspace(s3, bucket, client_id)
        .await
        .map_err(Into::into)
}

pub async fn rename_report_session(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    name: &str,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::rename_report_session(s3, bucket, client_id, report_id, name)
        .await
        .map_err(Into::into)
}

pub async fn save_report_draft(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    expected_revision: u64,
    content: ReportContent,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::save_report_draft(s3, bucket, client_id, expected_revision, content)
        .await
        .map_err(Into::into)
}

pub async fn save_report_draft_for_report(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    content: ReportContent,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::save_report_draft_for_report(
        s3,
        bucket,
        client_id,
        report_id,
        expected_revision,
        content,
    )
    .await
    .map_err(Into::into)
}

pub async fn store_report_template_source(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    source_sha256: &str,
    bytes: Vec<u8>,
) -> Result<(), ReportAuthoringError> {
    claria_report_store::store_report_template_source(s3, bucket, client_id, source_sha256, bytes)
        .await
        .map_err(Into::into)
}

pub async fn apply_report_template(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    expected_revision: u64,
    application: ReportTemplateApplication,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::apply_report_template(
        s3,
        bucket,
        client_id,
        expected_revision,
        application,
    )
    .await
    .map_err(Into::into)
}

pub async fn apply_report_template_for_report(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    application: ReportTemplateApplication,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::apply_report_template_for_report(
        s3,
        bucket,
        client_id,
        report_id,
        expected_revision,
        application,
    )
    .await
    .map_err(Into::into)
}

pub async fn resolve_report_proposal(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    proposal_id: Uuid,
    decision: ReportProposalDecision,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::resolve_report_proposal(s3, bucket, client_id, proposal_id, decision)
        .await
        .map_err(Into::into)
}

pub async fn resolve_report_proposal_for_report(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    proposal_id: Uuid,
    decision: ReportProposalDecision,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::resolve_report_proposal_for_report(
        s3,
        bucket,
        client_id,
        report_id,
        proposal_id,
        decision,
    )
    .await
    .map_err(Into::into)
}

pub async fn record_report_export(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    status: ReportExportStatus,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::record_report_export(s3, bucket, client_id, report_id, revision, status)
        .await
        .map_err(Into::into)
}

pub async fn load_export_snapshot(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
) -> Result<ReportExportSnapshot, ReportAuthoringError> {
    claria_report_store::load_export_snapshot(s3, bucket, client_id, report_id, expected_revision)
        .await
        .map_err(Into::into)
}

pub async fn delete_report_workspace_for_client(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<usize, ReportAuthoringError> {
    claria_report_store::delete_report_workspace_for_client(s3, bucket, client_id)
        .await
        .map_err(Into::into)
}

pub async fn restore_report_workspace_for_client(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
) -> Result<bool, ReportAuthoringError> {
    claria_report_store::restore_report_workspace_for_client(s3, bucket, client_id)
        .await
        .map_err(Into::into)
}

pub async fn list_report_revisions(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    cache: &RevisionCache,
) -> Result<Vec<ReportRevisionSummary>, ReportAuthoringError> {
    claria_report_store::list_report_revisions(s3, bucket, client_id, report_id, cache)
        .await
        .map_err(Into::into)
}

pub async fn load_report_revision(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    revision: u64,
    cache: &RevisionCache,
) -> Result<ReportDraft, ReportAuthoringError> {
    claria_report_store::load_report_revision(s3, bucket, client_id, report_id, revision, cache)
        .await
        .map_err(Into::into)
}

pub async fn revert_report_revision(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    revision: u64,
    cache: &RevisionCache,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::revert_report_revision(
        s3,
        bucket,
        client_id,
        report_id,
        expected_revision,
        revision,
        cache,
    )
    .await
    .map_err(Into::into)
}

pub async fn discard_queued_report_edits(
    s3: &S3Client,
    bucket: &str,
    client_id: Uuid,
    report_id: Uuid,
    expected_revision: u64,
    cache: &RevisionCache,
) -> Result<ReportWorkspace, ReportAuthoringError> {
    claria_report_store::discard_queued_report_edits(
        s3,
        bucket,
        client_id,
        report_id,
        expected_revision,
        cache,
    )
    .await
    .map_err(Into::into)
}
