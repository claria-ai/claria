use std::future::Future;
use std::pin::Pin;

use crate::error::ProvisionerError;
use crate::manifest::{FieldDrift, ResourceSpec};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One impl per resource type in the manifest.
/// Each impl holds its ResourceSpec + an AWS client.
pub trait ResourceSyncer: Send + Sync {
    /// The spec this syncer manages — carries all metadata.
    fn spec(&self) -> &ResourceSpec;

    /// Read current state from AWS. None = doesn't exist.
    fn read(&self) -> BoxFuture<'_, Result<Option<serde_json::Value>, ProvisionerError>>;

    /// Compare actual state (from read) against self.spec().desired.
    /// Returns empty vec if in sync, otherwise field-level drifts.
    fn diff(&self, actual: &serde_json::Value) -> Vec<FieldDrift>;

    /// Create the resource to match self.spec().desired.
    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>>;

    /// Update the resource to match self.spec().desired.
    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>>;

    /// Tear down the resource.
    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>>;
}
