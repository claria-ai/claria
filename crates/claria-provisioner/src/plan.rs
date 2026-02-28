use serde::{Deserialize, Serialize};
use specta::Type;

use crate::manifest::{FieldDrift, ResourceSpec};

/// A single entry in the plan — the spec annotated with what happened.
///
/// The plan is a flat `Vec<PlanEntry>` — same shape as the manifest array,
/// annotated with status. The entry embeds the full spec so the frontend
/// has everything it needs (label, description, severity, desired state)
/// without a separate lookup.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PlanEntry {
    pub spec: ResourceSpec,
    pub action: Action,
    pub cause: Cause,
    pub drift: Vec<FieldDrift>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Ok,
    Create,
    Modify,
    Delete,
    PreconditionFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    InSync,
    FirstProvision,
    Drift,
    ManifestChanged,
    Orphaned,
}
