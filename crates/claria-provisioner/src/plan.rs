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
    /// Live state read from AWS (if the resource exists).
    pub actual: Option<serde_json::Value>,
    /// Why the resource could not be read, when `action` is
    /// [`Action::Unknown`]. `None` for every other action.
    ///
    /// `#[serde(default)]` is the only optional-field spelling specta
    /// respects — a `skip_serializing_if` predicate here would make the
    /// generated binding required and disagree with the wire.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Ok,
    Create,
    Modify,
    Delete,
    PreconditionFailed,
    /// The read failed, so nothing is known about this resource. Never acted
    /// on: a resource Claria cannot see is not a resource it may create,
    /// change, or destroy.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    InSync,
    Missing,
    Drift,
    Orphaned,
    /// AWS refused or failed the read — permission, throttling, transport.
    /// Distinct from `Missing`, which is a resource that genuinely is not
    /// there.
    Unreadable,
}
