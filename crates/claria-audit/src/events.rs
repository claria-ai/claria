use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

/// A structured, application-level audit event.
///
/// CloudTrail records AWS API calls; these events record what the application
/// was *doing* — which record was extracted, what a chat turn cost, which
/// model answered it. Every event carries its own identity and UTC timestamp
/// so the persisted object is self-describing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub timestamp: Timestamp,
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
            event_id: Uuid::new_v4(),
            timestamp: Timestamp::now(),
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
    ///
    /// `details` is rendered as compact JSON and recorded as its own field.
    /// The desktop console layer flattens fields into the log line verbatim,
    /// so the payload survives as readable, greppable JSON rather than a
    /// `Some(Object {...})` debug rendering.
    pub fn emit(&self) {
        let details = match &self.details {
            Some(value) => value.to_string(),
            None => "{}".to_string(),
        };

        info!(
            audit.event_id = %self.event_id,
            audit.timestamp = %self.timestamp,
            audit.action = %self.action,
            audit.resource_type = %self.resource_type,
            audit.resource_id = %self.resource_id,
            audit.user_sub = %self.user_sub,
            audit.details = %details,
            "audit event"
        );
    }
}
