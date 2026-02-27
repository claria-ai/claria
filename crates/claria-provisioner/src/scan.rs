use serde::{Deserialize, Serialize};
use specta::Type;

use crate::resource::Resource;

/// The status of a single resource after scanning AWS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Found,
    NotFound,
    Error,
}

/// Result of scanning a single resource in AWS.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ScanResult {
    pub resource_type: String,
    pub status: ScanStatus,
    pub resource_id: Option<String>,
    pub properties: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Scan all resources and return their current state.
///
/// This is a pure read operation — no state required, no mutations.
pub async fn scan(resources: &[Box<dyn Resource>]) -> Vec<ScanResult> {
    let mut results = Vec::with_capacity(resources.len());

    for resource in resources {
        let resource_type = resource.resource_type().to_string();

        match resource.current_state().await {
            Ok(Some(props)) => {
                let resource_id = props
                    .get("resource_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| props.get("bucket_name").and_then(|v| v.as_str()))
                    .or_else(|| props.get("trail_arn").and_then(|v| v.as_str()))
                    .or_else(|| props.get("user_name").and_then(|v| v.as_str()))
                    .map(String::from);

                results.push(ScanResult {
                    resource_type,
                    status: ScanStatus::Found,
                    resource_id,
                    properties: Some(props),
                    error: None,
                });
            }
            Ok(None) => {
                results.push(ScanResult {
                    resource_type,
                    status: ScanStatus::NotFound,
                    resource_id: resource.expected_id().map(String::from),
                    properties: None,
                    error: None,
                });
            }
            Err(e) => {
                results.push(ScanResult {
                    resource_type,
                    status: ScanStatus::Error,
                    resource_id: resource.expected_id().map(String::from),
                    properties: None,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    results
}
