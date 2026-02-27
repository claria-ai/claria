use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Provisioner state, persisted to S3 at `_state/provisioner.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProvisionerState {
    /// Map of resource type -> resource ID -> resource state.
    pub resources: HashMap<String, ResourceState>,

    /// The AWS region this stack is deployed in.
    pub region: String,

    /// S3 bucket name.
    pub bucket: String,
}

/// State for a single managed resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceState {
    pub resource_type: String,
    pub resource_id: String,
    pub status: ResourceStatus,
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceStatus {
    Created,
    Updated,
    Deleted,
    Drifted,
    Unknown,
}
