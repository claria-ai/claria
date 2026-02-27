use serde::{Deserialize, Serialize};

/// An action to be taken on a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanAction {
    pub resource_type: String,
    pub resource_id: String,
    pub action: ActionType,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Create,
    Update,
    Delete,
    NoOp,
}

/// An execution plan listing all actions needed to reach desired state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub actions: Vec<PlanAction>,
}

impl ExecutionPlan {
    pub fn has_changes(&self) -> bool {
        self.actions
            .iter()
            .any(|a| a.action != ActionType::NoOp)
    }
}
