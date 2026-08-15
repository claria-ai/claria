//! S3 key/path conventions.
//!
//! Pure string functions — no AWS SDK dependency. These define the canonical
//! layout of objects in the Claria S3 bucket.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The one bucket Claria stores everything in, for a given AWS account and
/// system name.
///
/// The provisioner creates it under this exact name and the desktop app
/// derives it from saved config, so the two must never drift.
pub fn bucket_name(account_id: &str, system_name: &str) -> String {
    format!("{account_id}-{system_name}-data")
}

pub fn client(id: Uuid) -> String {
    format!("clients/{id}.json")
}

pub const CLIENTS_PREFIX: &str = "clients/";

pub fn client_records_prefix(id: Uuid) -> String {
    format!("records/{id}/")
}

/// ListObjectsV2 prefix for a client's record files whose filename starts
/// with `filename_prefix`.
pub fn client_records_search_prefix(id: Uuid, filename_prefix: &str) -> String {
    format!("records/{id}/{filename_prefix}")
}

/// True when `key` is a `.text` sidecar whose base file is present in `keys`.
///
/// `keys` must be the same (possibly prefix-filtered) listing `key` came from:
/// a sidecar stays hidden as long as its base file matches the filter too. A
/// prefix longer than the base filename (the user typed into the `.text`
/// suffix) excludes the base, so the sidecar is shown — it was asked for by
/// name.
pub fn is_hidden_sidecar(key: &str, keys: &std::collections::HashSet<&str>) -> bool {
    key.strip_suffix(".text")
        .is_some_and(|base| keys.contains(base))
}

/// A user-visible record file resolved against the sidecar rules, produced by
/// [`visible_record_files`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleRecordFile {
    /// Path relative to the `records/{uuid}/` prefix.
    pub filename: String,
    /// Index into the input `keys` slice of the file itself.
    pub base_index: usize,
    /// Index into the input `keys` slice of the object holding the file's
    /// readable text: its `.text` sidecar when one exists, otherwise the file
    /// itself.
    pub source_index: usize,
}

/// The one implementation of the record visibility rules, applied to a
/// complete `records/{uuid}/` listing:
///
/// - Chat history under `chat-history/` is not a record file.
/// - A `.text` sidecar is hidden while its base file exists (and is a visible
///   file of its own when the base is gone) — see [`is_hidden_sidecar`].
/// - Each visible file's readable text comes from its `.text` sidecar when
///   one exists, otherwise from the file itself.
///
/// `keys` must be the full listing under `prefix`; callers with S3 access
/// should go through `claria_records::record_inventory`, which walks the
/// bucket and applies this rule.
pub fn visible_record_files(prefix: &str, keys: &[&str]) -> Vec<VisibleRecordFile> {
    let index_by_key: std::collections::HashMap<&str, usize> = keys
        .iter()
        .enumerate()
        .map(|(index, key)| (*key, index))
        .collect();
    let key_set: std::collections::HashSet<&str> = keys.iter().copied().collect();

    keys.iter()
        .enumerate()
        .filter_map(|(base_index, key)| {
            let filename = key.strip_prefix(prefix)?;
            if filename.is_empty() || filename.starts_with("chat-history/") {
                return None;
            }
            if is_hidden_sidecar(key, &key_set) {
                return None;
            }
            let sidecar_key = format!("{key}.text");
            let source_index = index_by_key
                .get(sidecar_key.as_str())
                .copied()
                .unwrap_or(base_index);
            Some(VisibleRecordFile {
                filename: filename.to_string(),
                base_index,
                source_index,
            })
        })
        .collect()
}

pub fn client_record_file(id: Uuid, filename: &str) -> String {
    format!("records/{id}/{filename}")
}

pub fn chat_history_prefix(client_id: Uuid) -> String {
    format!("records/{client_id}/chat-history/")
}

pub fn chat_history(client_id: Uuid, chat_id: Uuid) -> String {
    format!("records/{client_id}/chat-history/{chat_id}.json")
}

/// Root for the opt-in report-authoring workspace. It is intentionally
/// separate from `records/` so report state cannot appear in record listings
/// or the existing Chat context.
pub const REPORT_AUTHORING_PREFIX: &str = "report-authoring/";

pub fn report_authoring_client_prefix(client_id: Uuid) -> String {
    format!("{REPORT_AUTHORING_PREFIX}{client_id}/")
}

/// Legacy singleton writer workspace key. Existing installations may still
/// have one session here; new sessions use [`report_session_workspace`].
pub fn report_workspace(client_id: Uuid) -> String {
    format!(
        "{}workspace.json",
        report_authoring_client_prefix(client_id)
    )
}

pub fn report_sessions_prefix(client_id: Uuid) -> String {
    format!("{}sessions/", report_authoring_client_prefix(client_id))
}

/// One independently resumable Writing session, analogous to a saved chat.
pub fn report_session_workspace(client_id: Uuid, report_id: Uuid) -> String {
    format!("{}{report_id}.json", report_sessions_prefix(client_id))
}

pub fn report_attempt(client_id: Uuid, attempt_id: Uuid) -> String {
    format!(
        "{}attempts/{attempt_id}.json",
        report_authoring_client_prefix(client_id)
    )
}

pub fn report_call_usage(client_id: Uuid, attempt_id: Uuid, call_number: u32) -> String {
    format!(
        "{}usage/{attempt_id}/{call_number}.json",
        report_authoring_client_prefix(client_id)
    )
}

pub fn report_template_source(client_id: Uuid, source_sha256: &str) -> String {
    format!(
        "{}templates/{source_sha256}.docx",
        report_authoring_client_prefix(client_id)
    )
}

pub const WRITER_TEMPLATES_PREFIX: &str = "writer_templates/";

pub fn writer_template_docx(template_id: Uuid) -> String {
    format!("{WRITER_TEMPLATES_PREFIX}{template_id}.docx")
}

pub fn writer_template_metadata(template_id: Uuid) -> String {
    format!("{WRITER_TEMPLATES_PREFIX}{template_id}.json")
}

pub fn writer_template_usage(template_id: Uuid) -> String {
    format!("{WRITER_TEMPLATES_PREFIX}{template_id}.usage.json")
}

pub fn client_lifecycle(client_id: Uuid) -> String {
    format!("_state/client-lifecycle/{client_id}.json")
}

pub const PROMPTS_PREFIX: &str = "claria-prompts/";

pub const SYSTEM_PROMPT: &str = "claria-prompts/system-prompt.md";

pub const EXTRACTION_PROMPT: &str = "claria-prompts/pdf-extraction.md";

/// Customized body of the writer's targeted-edit system prompt.
pub const REPORT_SYSTEM_PROMPT: &str = "claria-prompts/report-system-prompt.md";

/// Customized body of the writer's whole-document system prompt.
pub const FULL_REPORT_SYSTEM_PROMPT: &str = "claria-prompts/full-report-system-prompt.md";

/// Legacy key for the system prompt before the `claria-prompts/` migration.
/// Used as a read fallback so existing buckets keep working.
pub const LEGACY_SYSTEM_PROMPT: &str = "system-prompt.md";

/// Reusable writer steering prompts the user picks to prefill an
/// instruction, one JSON object per prompt.
pub const WRITER_PROMPT_LIBRARY_PREFIX: &str = "claria-prompts/writer-library/";

pub fn writer_library_prompt(prompt_id: Uuid) -> String {
    format!("{WRITER_PROMPT_LIBRARY_PREFIX}{prompt_id}.json")
}

pub const PROVISIONER_STATE: &str = "_state/provisioner.json";

/// Scratch space for Amazon Transcribe job output. Written by Transcribe
/// itself, read once, then deleted — nothing here outlives a transcription.
pub fn transcribe_output(job_name: &str) -> String {
    format!("_transcribe/{job_name}.json")
}

pub const PREFERENCES: &str = "_state/preferences.json";

/// A PHI-safe rendering of an S3 key (or listing prefix) for log fields.
///
/// Client-chosen record filenames can identify a person, so anything after
/// `records/{uuid}/` collapses to `<file>` (keeping a `.text` sidecar suffix
/// for debuggability) — except the `chat-history/` folder, whose entries are
/// UUID-named and therefore safe. Every other layout in the bucket is
/// app-generated (UUIDs, hashes, fixed names) and passes through unchanged.
pub fn log_safe_key(key: &str) -> String {
    let mut parts = key.splitn(3, '/');
    if parts.next() == Some("records")
        && let Some(client_segment) = parts.next()
        && let Some(rest) = parts.next()
        && !rest.is_empty()
        && !rest.starts_with("chat-history/")
    {
        if rest.ends_with(".text") {
            return format!("records/{client_segment}/<file>.text");
        }
        return format!("records/{client_segment}/<file>");
    }
    key.to_string()
}

// ── Application audit trail ─────────────────────────────────────────────────

/// Root of the application audit trail. Everything below it is written once
/// and never modified.
pub const AUDIT_PREFIX: &str = "_audit/";

/// Prefix covering every audit event recorded in a UTC month.
pub fn audit_month_prefix(year: i16, month: i8) -> String {
    format!("_audit/{year:04}/{month:02}/")
}

/// Prefix covering every audit event recorded on a UTC day.
pub fn audit_day_prefix(date: jiff::civil::Date) -> String {
    format!(
        "_audit/{:04}/{:02}/{:02}/",
        date.year(),
        date.month(),
        date.day()
    )
}

/// Key for a single audit event.
///
/// The UTC date is spread across three path segments so an auditor can list a
/// day, a month, or a year without walking the whole trail. Within a day the
/// filename leads with a fixed-width UTC timestamp at nanosecond precision, so
/// S3's lexicographic listing order is also chronological order; the event's
/// UUID follows to keep two events in the same nanosecond from colliding.
pub fn audit_event(timestamp: jiff::Timestamp, event_id: Uuid) -> String {
    let dt = timestamp.to_zoned(jiff::tz::TimeZone::UTC).datetime();
    format!(
        "_audit/{year:04}/{month:02}/{day:02}/\
         {year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}.{nanos:09}Z-{event_id}.json",
        year = dt.year(),
        month = dt.month(),
        day = dt.day(),
        hour = dt.hour(),
        minute = dt.minute(),
        second = dt.second(),
        nanos = dt.subsec_nanosecond(),
    )
}
