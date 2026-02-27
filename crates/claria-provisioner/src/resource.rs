use std::future::Future;
use std::pin::Pin;

use crate::error::ProvisionerError;

/// Result of a resource create or update operation.
pub struct ResourceResult {
    pub resource_id: String,
    pub properties: serde_json::Value,
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Trait implemented by each managed AWS resource.
///
/// Each resource type (S3 bucket, Cognito pool, Lambda function, etc.)
/// implements this trait to describe, create, update, and delete itself.
///
/// Methods return boxed futures for dyn compatibility.
pub trait Resource: Send + Sync {
    /// The resource type identifier (e.g. "s3_bucket", "cognito_user_pool").
    fn resource_type(&self) -> &str;

    /// The expected resource name or identifier, even if the resource doesn't
    /// exist yet. Used to show the user which resource will be
    /// created/checked (e.g. the bucket name or trail name).
    fn expected_id(&self) -> Option<&str> {
        None
    }

    /// Query the current state of this resource in AWS.
    /// Returns `Ok(None)` if the resource doesn't exist yet.
    fn current_state(
        &self,
    ) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>>;

    /// Create the resource in AWS.
    fn create(&self) -> BoxFuture<'_, Result<ResourceResult, ProvisionerError>>;

    /// Update the resource to match desired state.
    fn update(
        &self,
        resource_id: &str,
    ) -> BoxFuture<'_, Result<ResourceResult, ProvisionerError>>;

    /// Delete the resource from AWS.
    fn delete(&self, resource_id: &str) -> BoxFuture<'_, Result<(), ProvisionerError>>;
}
