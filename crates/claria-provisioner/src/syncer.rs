use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

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

    /// The desired state for drift comparison.
    ///
    /// Default: `self.spec().desired.clone()`. Override when the desired
    /// state needs transformation (e.g. rendering a policy document).
    fn desired_state(&self) -> Value {
        self.spec().desired.clone()
    }

    /// Extract the comparison-relevant current state from the raw read.
    ///
    /// Default: `actual.clone()`. Override when `read()` returns extra
    /// fields beyond what's needed for comparison (e.g. stripping ARNs).
    fn current_state(&self, actual: &Value) -> Value {
        actual.clone()
    }

    /// Create the resource to match self.spec().desired.
    fn create(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>>;

    /// Update the resource to match self.spec().desired.
    fn update(&self) -> BoxFuture<'_, Result<serde_json::Value, ProvisionerError>>;

    /// Tear down the resource.
    fn destroy(&self) -> BoxFuture<'_, Result<(), ProvisionerError>>;
}

/// Compare desired vs current JSON values, producing field-level drift.
///
/// If both are objects, diffs field-by-field. Otherwise compares as a single value.
pub fn compute_drift(desired: &Value, current: &Value) -> Vec<FieldDrift> {
    if desired == current {
        return vec![];
    }

    if let (Some(d), Some(c)) = (desired.as_object(), current.as_object()) {
        let mut keys: Vec<&String> = d
            .keys()
            .chain(c.keys())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        keys.sort();
        return keys
            .into_iter()
            .filter_map(|key| {
                let d_val = d.get(key).unwrap_or(&Value::Null);
                let c_val = c.get(key).unwrap_or(&Value::Null);
                if d_val == c_val {
                    return None;
                }
                Some(FieldDrift {
                    field: key.clone(),
                    label: humanize_field_name(key),
                    expected: d_val.clone(),
                    actual: c_val.clone(),
                })
            })
            .collect();
    }

    vec![FieldDrift {
        field: "value".into(),
        label: "Value".into(),
        expected: desired.clone(),
        actual: current.clone(),
    }]
}

fn humanize_field_name(field: &str) -> String {
    let mut result = field.replace('_', " ");
    if let Some(first) = result.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    result
}
