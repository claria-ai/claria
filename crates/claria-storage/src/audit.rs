//! Application audit events and the durable S3 sink they are written to.
//!
//! This lives in `claria-storage` because the trail is nothing but S3 writes
//! and this crate owns those; every consumer of the events (the desktop's
//! never-fail `audit::record` wrapper) already depends on storage.
//!
//! S3 has no append, so the trail is one immutable object per event rather
//! than a growing log file. Events are human-paced (a chat turn, a document
//! extraction), so the per-event PUT is cheap next to the Bedrock call that
//! produced it, and nothing is ever buffered in memory waiting to be written —
//! a record either exists in S3 or the write failed loudly.
//!
//! # What an event is for
//!
//! The trail is a PHI-access record first and a cost ledger second. Its
//! primary question is "who read which client's record, and when" — cost
//! attribution rides along on the [`EventCategory::Ai`] category because
//! those events happen to know it.
//!
//! # `details` versus `phi`
//!
//! Every event has two payload slots, and which one a value goes in decides
//! whether it can ever appear in an exported log:
//!
//! - [`AuditEvent::details`] is **console-safe by contract**. Counts, byte
//!   sizes, token counts, dollar cost, model ids, version ids, durations,
//!   booleans. Machine-shaped values drawn from a bounded vocabulary.
//! - [`AuditEvent::phi`] is **never emitted to tracing**. Search query text,
//!   client names, filenames, transcript fragments — anything free-text or
//!   name-shaped. It is serialized to S3 in full and dropped on the floor by
//!   [`AuditEvent::emit`].
//!
//! When in doubt a value is `phi`. The cost of over-classifying is a slightly
//! thinner console log; the cost of under-classifying is PHI in a file the
//! user emails to support.
//!
//! [`AuditEvent::resource_id`] is treated the same way, because it can hold a
//! filename. `emit` omits it wholesale rather than asking each call site to
//! judge whether its own resource id is safe.
//!
//! # The two sinks
//!
//! The S3 trail under `_audit/` keeps full fidelity and is PHI-bearing by
//! design. The Claria Console — the in-memory ring buffer the user can export
//! — carries a fixed, safe subset. This is not scrub-on-export: the desktop
//! console layer flattens every field of every admitted tracing event into a
//! single message string, so by the time an event reaches the buffer there is
//! no structured boundary left to filter on. The filter therefore lives at
//! the emit site, in [`AuditEvent::emit`], and is a fixed allow-list.
//!
//! # Action taxonomy
//!
//! Actions are `{noun}.{verb}` names drawn from [`actions`], and each one
//! carries its own [`EventCategory`]. Because [`AuditEvent::new`] takes an
//! [`Action`] rather than a string, an action and its category cannot drift
//! apart, and a call site cannot invent an action the taxonomy does not know.
//!
//! ## The pre-v2 trail is not readable
//!
//! Schema v2 is a clean break. `category` is required, so an object written
//! before v2 fails to deserialize rather than degrading to a partial event,
//! and [`read_day`] fails for any day that contains one. The trail was days
//! old with a single consumer when this landed, so the alternative — an
//! `Unspecified` category and unmapped action names carried for the whole
//! six-year retention window, with every query needing two spellings — cost
//! more than deleting a handful of objects.
//!
//! Purge `_audit/` of pre-v2 objects when deploying this. The actions that
//! were renamed: `chat_message`, `infra_chat`, `extract_document_text`,
//! `translate_transcript`, `save_transcript_edits`, and every action the
//! report-authoring and preferences surfaces wrote as a bare string —
//! `draft_plan_generated`, `draft_plan_edited`, `draft_run_started`,
//! `draft_run_resumed`, `draft_run_stopped`, `draft_run_abandoned`,
//! `draft_run_finalized_partial`, `review_sweep_completed`,
//! `finding_applied`, `finding_undone`, `finding_dismissed`,
//! `preferences_imported`, `preferences_version_restored` — which now carry
//! the `{noun}.{verb}` spelling and a category like everything else.
//!
//! # When per-operator identity arrives
//!
//! Claria today authenticates a machine, not a person: `user_sub` carries the
//! AWS account id and `credential_id` carries the access key id (or profile
//! name) that machine uses. Because the IAM two-key limit already forces one
//! key per computer, that pair gives per-machine attribution now.
//!
//! When real per-operator identity lands, **nothing here needs a new field**.
//! `user_sub` simply starts carrying an operator identity instead of an
//! account id, and `credential_id` keeps carrying the credential — the two
//! answer different questions ("who" and "from which machine") and both stay
//! useful. The choke point is [`AuditEvent::new`], not the call sites that
//! reach it, and neither the S3 key layout nor the category taxonomy mentions
//! identity at all. Do not invent a parallel operator field.

use aws_config::SdkConfig;
use claria_core::s3_keys;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StorageError;

/// What kind of thing an event records.
///
/// The category is what makes "show me every read of this client's records"
/// a filter, rather than a list of action names that has to be kept in sync
/// with the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    /// A read of stored PHI.
    Access,
    /// A create, update, delete or restore.
    Mutation,
    /// A model or Amazon Transcribe invocation. Carries cost fields.
    Ai,
    /// A non-PHI operational action.
    Admin,
}

impl EventCategory {
    /// The wire spelling, for log rendering.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Mutation => "mutation",
            Self::Ai => "ai",
            Self::Admin => "admin",
        }
    }
}

impl std::fmt::Display for EventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One action name, bound to the category it belongs to.
///
/// Call sites pass an [`actions`] constant rather than a string, so a typo is
/// a compile error instead of an event no query will ever match, and the
/// category cannot be chosen independently of the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Action {
    /// The `{noun}.{verb}` name written to the trail.
    pub name: &'static str,
    /// The category every event with this action carries.
    pub category: EventCategory,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

/// Every action name the trail may record, each bound to its category.
pub mod actions {
    use super::{Action, EventCategory};

    const fn access(name: &'static str) -> Action {
        Action {
            name,
            category: EventCategory::Access,
        }
    }
    const fn mutation(name: &'static str) -> Action {
        Action {
            name,
            category: EventCategory::Mutation,
        }
    }
    const fn ai(name: &'static str) -> Action {
        Action {
            name,
            category: EventCategory::Ai,
        }
    }
    const fn admin(name: &'static str) -> Action {
        Action {
            name,
            category: EventCategory::Admin,
        }
    }

    // -- access: a read of stored PHI ------------------------------------
    /// A record file's bytes were fetched.
    pub const RECORD_FILE_READ: Action = access("record.file_read");
    /// A record's extracted text was pulled into a chat context.
    pub const RECORD_CONTEXT_READ: Action = access("record.context_read");
    /// A search was run over client records. The query text is `phi`.
    pub const RECORD_SEARCH: Action = access("record.search");
    /// A prior version of a record file was fetched.
    pub const RECORD_VERSION_READ: Action = access("record.version_read");
    /// The deleted-files list for a client was listed.
    pub const RECORD_DELETED_LIST: Action = access("record.deleted_list");
    /// A persisted chat session was loaded.
    pub const CHAT_HISTORY_READ: Action = access("chat.history_read");
    /// The deleted-clients list was listed.
    pub const CLIENT_DELETED_LIST: Action = access("client.deleted_list");
    /// A report was exported to a DOCX file on the clinician's machine.
    /// Categorised as a read: it moves PHI out of the bucket without
    /// changing anything in it.
    pub const REPORT_DOCX_EXPORT: Action = access("report.docx_export");

    // -- mutation: create, update, delete, restore -----------------------
    /// A file was uploaded to a client's record.
    pub const RECORD_FILE_UPLOAD: Action = mutation("record.file_upload");
    /// A record file was created in the app rather than uploaded.
    pub const RECORD_FILE_CREATE: Action = mutation("record.file_create");
    /// A record file's contents were replaced.
    pub const RECORD_FILE_UPDATE: Action = mutation("record.file_update");
    /// A transcript sidecar was edited and saved.
    pub const RECORD_TRANSCRIPT_EDIT: Action = mutation("record.transcript_edit");
    /// A record file was deleted.
    pub const RECORD_FILE_DELETE: Action = mutation("record.file_delete");
    /// A deleted record file was restored.
    pub const RECORD_FILE_RESTORE: Action = mutation("record.file_restore");
    /// A record file was rolled back to a prior version.
    pub const RECORD_VERSION_RESTORE: Action = mutation("record.version_restore");
    /// A client record was created.
    pub const CLIENT_CREATE: Action = mutation("client.create");
    /// A client record was renamed.
    pub const CLIENT_RENAME: Action = mutation("client.rename");
    /// A client record was deleted.
    pub const CLIENT_DELETE: Action = mutation("client.delete");
    /// A deleted client record was restored.
    pub const CLIENT_RESTORE: Action = mutation("client.restore");
    /// A persisted chat session was renamed.
    pub const CHAT_HISTORY_RENAME: Action = mutation("chat.history_rename");

    // -- mutation: writer templates and report authoring -----------------
    /// A writer template was uploaded.
    pub const WRITER_TEMPLATE_UPLOAD: Action = mutation("writer_template.upload");
    /// A writer template was renamed.
    pub const WRITER_TEMPLATE_RENAME: Action = mutation("writer_template.rename");
    /// A writer template was deleted.
    pub const WRITER_TEMPLATE_DELETE: Action = mutation("writer_template.delete");
    /// A DOCX was imported as a report's formatting template.
    pub const REPORT_TEMPLATE_IMPORT: Action = mutation("report.template_import");
    /// A named writer session was renamed.
    pub const REPORT_SESSION_RENAME: Action = mutation("report.session_rename");
    /// A report was rolled back to an earlier revision.
    pub const REPORT_REVISION_RESTORE: Action = mutation("report.revision_restore");
    /// Queued writer edits were discarded without being applied.
    pub const REPORT_QUEUED_EDITS_DISCARD: Action = mutation("report.queued_edits_discard");
    /// A report draft was written back to the workspace.
    pub const REPORT_DRAFT_SAVE: Action = mutation("report.draft_save");
    /// A writer proposal was accepted into the report.
    pub const REPORT_PROPOSAL_ACCEPT: Action = mutation("report.proposal_accept");
    /// A writer proposal was rejected.
    pub const REPORT_PROPOSAL_REJECT: Action = mutation("report.proposal_reject");
    /// The section plan for a whole-report draft was edited by hand before
    /// the run was let off the gate.
    pub const REPORT_DRAFT_PLAN_EDIT: Action = mutation("report.draft_plan_edit");
    /// A gated draft run was released and began drafting.
    pub const REPORT_DRAFT_RUN_START: Action = mutation("report.draft_run_start");
    /// A parked draft run picked up the sections it had not reached.
    pub const REPORT_DRAFT_RUN_RESUME: Action = mutation("report.draft_run_resume");
    /// A partial draft run was cut into a revision as-is, leaving its
    /// unreached sections undrafted.
    pub const REPORT_DRAFT_RUN_FINALIZE_PARTIAL: Action =
        mutation("report.draft_run_finalize_partial");
    /// A draft run was thrown away without being finalized.
    pub const REPORT_DRAFT_RUN_ABANDON: Action = mutation("report.draft_run_abandon");
    /// A review finding was applied to the report.
    pub const REPORT_FINDING_APPLY: Action = mutation("report.finding_apply");
    /// An applied review finding was undone.
    pub const REPORT_FINDING_UNDO: Action = mutation("report.finding_undo");
    /// A review finding was dismissed without being applied.
    pub const REPORT_FINDING_DISMISS: Action = mutation("report.finding_dismiss");

    // -- ai: a model or Transcribe invocation, carries cost --------------
    /// One chat turn against a client's records.
    pub const CHAT_TURN: Action = ai("chat.turn");
    /// Text extracted from a PDF or DOCX by a model.
    pub const RECORD_EXTRACT_TEXT: Action = ai("record.extract_text");
    /// Transcript segments translated to English by a model.
    pub const RECORD_TRANSLATE: Action = ai("record.translate");
    /// Audio transcribed by Amazon Transcribe.
    pub const RECORD_TRANSCRIBE: Action = ai("record.transcribe");
    /// One chat turn against the infrastructure assistant. No PHI involved,
    /// but it spends money, so it is categorised by what it invokes.
    pub const INFRA_CHAT: Action = ai("infra.chat");
    /// A whole-report draft was generated by a model.
    pub const REPORT_FULL_DRAFT: Action = ai("report.full_draft");
    /// A whole-report generation failed. Recorded because the spend happened
    /// whether or not a draft came back.
    pub const REPORT_FULL_DRAFT_FAILED: Action = ai("report.full_draft_failed");
    /// One tool-use round of the agentic writer loop.
    pub const REPORT_TOOL_TURN: Action = ai("report.tool_turn");
    /// A writer tool-use round that failed, with the spend it still incurred.
    pub const REPORT_TOOL_TURN_FAILED: Action = ai("report.tool_turn_failed");
    /// A whole-report draft the user stopped. It changed nothing, but it
    /// spent tokens before it was cut, so the receipt keeps that traceable.
    pub const REPORT_DRAFT_RUN_STOPPED: Action = ai("report.draft_run_stopped");
    /// A writer tool-use round the user stopped, with the spend it incurred.
    pub const REPORT_TOOL_TURN_STOPPED: Action = ai("report.tool_turn_stopped");
    /// A section plan was generated for a whole-report draft.
    pub const REPORT_DRAFT_PLAN_GENERATE: Action = ai("report.draft_plan_generate");
    /// A review sweep ran every property against an accepted revision.
    pub const REPORT_REVIEW_SWEEP: Action = ai("report.review_sweep");

    // -- admin: non-PHI operational --------------------------------------
    /// A custom prompt was saved.
    pub const PROMPT_SAVE: Action = admin("prompt.save");
    /// A custom prompt was deleted.
    pub const PROMPT_DELETE: Action = admin("prompt.delete");
    /// A custom prompt was reset to the built-in default.
    pub const PROMPT_RESTORE: Action = admin("prompt.restore");
    /// A preferences bundle was imported over the current settings. The
    /// bundle is machine configuration, not client data.
    pub const PREFERENCES_IMPORT: Action = admin("preferences.import");
    /// Preferences were rolled back to an earlier stored version.
    pub const PREFERENCES_VERSION_RESTORE: Action = admin("preferences.version_restore");
    /// A saved writer steering prompt was created. These are shared across
    /// clients and are meant to carry placeholders rather than client
    /// details, which is why they are administrative and not a record
    /// mutation.
    pub const WRITER_PROMPT_CREATE: Action = admin("writer_prompt.create");
    /// A saved writer steering prompt was edited.
    pub const WRITER_PROMPT_UPDATE: Action = admin("writer_prompt.update");
    /// A saved writer steering prompt was deleted.
    pub const WRITER_PROMPT_DELETE: Action = admin("writer_prompt.delete");
    /// The Claria Console log was exported to a file.
    pub const CONSOLE_EXPORT: Action = admin("console.export");
}

/// A structured, application-level audit event. Schema v2.
///
/// CloudTrail records AWS API calls; these events record what the application
/// was *doing* — which record was read, what a chat turn cost, which model
/// answered it. Every event carries its own identity and UTC timestamp so the
/// persisted object is self-describing.
///
/// See the [module docs](self) for the `details`-versus-`phi` contract that
/// every field below is placed against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub timestamp: Timestamp,
    pub action: String,
    /// Which kind of action this is. Required: an object without it is not a
    /// v2 event and does not read at all.
    pub category: EventCategory,
    pub resource_type: String,
    /// Identifies the resource acted on. For record files this is the S3 key
    /// minus the `records/` prefix — `{client_uuid}/{filename}`. Because it
    /// can carry a filename it is never emitted to tracing.
    pub resource_id: String,
    /// First-class so per-client filtering never has to parse `details`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<Uuid>,
    /// Who acted. Today the AWS account id; see the module docs on identity.
    pub user_sub: String,
    /// Which credential acted: the AWS access key id for an inline
    /// credential, the profile name for a named profile, `None` for the
    /// default chain. Access key ids are identifiers, not secrets —
    /// CloudTrail records them on every call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// Console-safe payload: counts, sizes, token counts, cost, model ids,
    /// version ids.
    pub details: Option<serde_json::Value>,
    /// PHI-bearing payload. Written to S3, never emitted to tracing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phi: Option<serde_json::Value>,
    /// The app version that recorded the event. The caller stamps it — this
    /// crate never reads its own version, so library and desktop releases
    /// stay independent.
    #[serde(default)]
    pub app_version: Option<String>,
}

impl AuditEvent {
    /// Build an event. `action` carries its own category, so the two cannot
    /// disagree and there is no way to reach a category the taxonomy did not
    /// assign.
    pub fn new(
        action: Action,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        user_sub: impl Into<String>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Timestamp::now(),
            action: action.name.to_string(),
            category: action.category,
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            client_id: None,
            user_sub: user_sub.into(),
            credential_id: None,
            details: None,
            phi: None,
            app_version: None,
        }
    }

    /// Attach the console-safe payload. See the [module docs](self).
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Attach the PHI-bearing payload. Reaches S3; never reaches the console.
    pub fn with_phi(mut self, phi: serde_json::Value) -> Self {
        self.phi = Some(phi);
        self
    }

    /// Attach the client this event concerns.
    pub fn with_client_id(mut self, client_id: Uuid) -> Self {
        self.client_id = Some(client_id);
        self
    }

    /// Attach the credential that acted. `None` for the default chain.
    pub fn with_credential_id(mut self, credential_id: Option<String>) -> Self {
        self.credential_id = credential_id;
        self
    }

    pub fn with_app_version(mut self, app_version: impl Into<String>) -> Self {
        self.app_version = Some(app_version.into());
        self
    }

    /// Mirror the console-safe subset of this event into the tracing stream.
    ///
    /// The fields below are a fixed allow-list, not a filter over the event:
    /// `resource_id` and `phi` are dropped wholesale. `resource_id` can hold a
    /// filename, and a uniform rule leaves no per-event judgment for a future
    /// call site to get wrong. The console export is a support artifact in a
    /// HIPAA app. The dedicated target keeps the trail greppable.
    ///
    /// `details` is rendered as compact JSON so an auditor grepping the
    /// exported log gets a parseable line rather than a Rust debug rendering.
    pub fn emit(&self) {
        let details = match &self.details {
            Some(value) => value.to_string(),
            None => "{}".to_string(),
        };
        let client_id = match &self.client_id {
            Some(id) => id.to_string(),
            None => String::new(),
        };

        // `event!` rather than `info!`: tracing 0.1.44's `info!` macro cannot
        // parse `target:` together with dotted field names.
        tracing::event!(
            target: "claria_storage::audit::trail",
            tracing::Level::INFO,
            audit.event_id = %self.event_id,
            audit.timestamp = %self.timestamp,
            audit.action = %self.action,
            audit.category = %self.category,
            audit.resource_type = %self.resource_type,
            audit.client_id = %client_id,
            audit.user_sub = %self.user_sub,
            audit.details = %details,
            "audit event"
        );
    }
}

/// Write one audit event to S3 as its own object. Returns the key written.
///
/// The key embeds the event's own timestamp, so retries and out-of-order
/// writes still land in the right time slice.
#[tracing::instrument(level = "trace", skip_all, fields(bucket = %bucket, action = %event.action))]
pub async fn write_event(
    sdk_config: &SdkConfig,
    bucket: &str,
    event: &AuditEvent,
) -> Result<String, StorageError> {
    let key = s3_keys::audit_event(event.timestamp, event.event_id);
    let body = serde_json::to_vec(event)?;

    let s3 = crate::client::from_config(sdk_config);
    crate::objects::put_object(&s3, bucket, &key, body, Some("application/json")).await?;

    Ok(key)
}

/// Read every audit event recorded on `date` (UTC), oldest first.
///
/// The day prefix is what makes "what happened on this date" a single
/// `ListObjectsV2`, and the key layout makes S3's listing order chronological,
/// so no sorting is needed after the fact. The per-event GETs fan out with
/// bounded concurrency; `buffered` yields them back in listing order.
pub async fn read_day(
    sdk_config: &SdkConfig,
    bucket: &str,
    date: jiff::civil::Date,
) -> Result<Vec<AuditEvent>, StorageError> {
    use futures::stream::StreamExt;

    let s3 = crate::client::from_config(sdk_config);
    let prefix = s3_keys::audit_day_prefix(date);
    let keys = crate::objects::list_objects(&s3, bucket, &prefix).await?;

    let s3 = &s3;
    let fetches = keys.iter().map(|key| async move {
        let object = crate::objects::get_object(s3, bucket, key).await?;
        serde_json::from_slice::<AuditEvent>(&object.body).map_err(StorageError::from)
    });
    futures::stream::iter(fetches)
        .buffered(crate::objects::S3_FETCH_CONCURRENCY)
        .collect::<Vec<Result<AuditEvent, StorageError>>>()
        .await
        .into_iter()
        .collect()
}
