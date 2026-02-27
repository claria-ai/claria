use serde::Serialize;
use tracing::info;

/// A structured audit event for logging API actions.
///
/// These events are logged via `tracing` so they appear in CloudWatch Logs.
/// CloudTrail captures the underlying AWS API calls automatically; these
/// application-level events provide higher-level context.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub user_sub: String,
    pub details: Option<serde_json::Value>,
}

impl AuditEvent {
    pub fn new(
        action: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        user_sub: impl Into<String>,
    ) -> Self {
        Self {
            action: action.into(),
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            user_sub: user_sub.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Emit this audit event via tracing.
    pub fn emit(&self) {
        info!(
            audit.action = %self.action,
            audit.resource_type = %self.resource_type,
            audit.resource_id = %self.resource_id,
            audit.user_sub = %self.user_sub,
            "audit event"
        );
    }
}
