use serde::{Deserialize, Serialize};
use specta::Type;

/// A single entry in a provisioning plan.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PlanEntry {
    pub resource_type: String,
    pub resource_id: String,
    pub reason: String,
}

/// A provisioning plan with four categorized buckets.
///
/// The desktop UI renders these as color-coded lists:
/// - `ok` (green) — resources in good shape, no action needed
/// - `modify` (yellow) — resources that need updating (e.g. missing encryption)
/// - `create` (blue) — resources that don't exist yet
/// - `delete` (red) — stale state entries to clean up
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct Plan {
    pub ok: Vec<PlanEntry>,
    pub modify: Vec<PlanEntry>,
    pub create: Vec<PlanEntry>,
    pub delete: Vec<PlanEntry>,
}

impl Plan {
    /// Returns true if the plan requires any changes.
    pub fn has_changes(&self) -> bool {
        !self.modify.is_empty() || !self.create.is_empty() || !self.delete.is_empty()
    }
}
