use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::addr::ResourceAddr;

/// Provisioner state, persisted to S3 at `_state/provisioner.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProvisionerState {
    /// Map of resource address -> resource state.
    pub resources: HashMap<ResourceAddr, ResourceState>,

    /// Manifest version at time of last successful execute.
    /// None on first run or migrated state.
    #[serde(default)]
    pub manifest_version: Option<u32>,

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

/// Migrate v1 state (keyed by resource_type string) to v2 (keyed by ResourceAddr).
///
/// Old format: `{"resources": {"s3_bucket": {resource_id: "123-claria-data", ...}}}`
/// New format: `{"resources": {"s3_bucket.123-claria-data": {...}}, "manifest_version": null}`
pub fn migrate_state_v1_to_v2(old: serde_json::Value) -> serde_json::Value {
    let Some(obj) = old.as_object() else {
        return old;
    };

    // If the state already has manifest_version, it's already v2 or later
    if obj.contains_key("manifest_version") {
        return old;
    }

    let mut new = obj.clone();

    if let Some(serde_json::Value::Object(resources)) = obj.get("resources") {
        let mut new_resources = serde_json::Map::new();
        for (resource_type, state) in resources {
            // Infer resource_name from resource_id in the state
            let resource_name = state
                .get("resource_id")
                .and_then(|v| v.as_str())
                .unwrap_or(resource_type)
                .to_string();

            let addr = ResourceAddr {
                resource_type: resource_type.clone(),
                resource_name,
            };
            let key = format!("{}", addr);
            new_resources.insert(key, state.clone());
        }
        new.insert(
            "resources".to_string(),
            serde_json::Value::Object(new_resources),
        );
    }

    // manifest_version defaults to None via #[serde(default)]
    serde_json::Value::Object(new)
}
