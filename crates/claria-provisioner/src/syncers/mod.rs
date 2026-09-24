use aws_smithy_types::error::display::DisplayErrorContext;

use crate::error::ProvisionerError;

pub mod baa_agreement;
pub mod bedrock_model_agreement;
pub mod cloudtrail_trail;
pub mod cloudtrail_trail_logging;
pub mod cost_explorer_access;
pub mod iam_user;
pub mod iam_user_policy;
pub mod s3_bucket;
pub mod s3_bucket_encryption;
pub mod s3_bucket_policy;
pub mod s3_bucket_public_access_block;
pub mod s3_bucket_versioning;
pub mod transcribe_access;

/// The error a read that did not happen reports.
///
/// Every syncer's `read` uses this for anything that is not a resource
/// genuinely absent, so the plan can tell "it is not there" from "I was not
/// allowed to look". The operation name is what makes a refused call findable
/// in a console export.
///
/// [`DisplayErrorContext`] rather than `into_service_error().to_string()`: the
/// latter turns a network failure into "unhandled error".
pub(crate) fn read_failed<E>(operation: &str, error: &E) -> ProvisionerError
where
    E: std::error::Error + 'static,
{
    ProvisionerError::Aws(format!(
        "{operation} failed: {}",
        DisplayErrorContext(error)
    ))
}
