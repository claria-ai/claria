//! S3 key/path conventions.
//!
//! Pure string functions — no AWS SDK dependency. These define the canonical
//! layout of objects in the Claria S3 bucket.

use uuid::Uuid;

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

pub fn client_record_file(id: Uuid, filename: &str) -> String {
    format!("records/{id}/{filename}")
}

pub fn chat_history_prefix(client_id: Uuid) -> String {
    format!("records/{client_id}/chat-history/")
}

pub fn chat_history(client_id: Uuid, chat_id: Uuid) -> String {
    format!("records/{client_id}/chat-history/{chat_id}.json")
}

pub const PROMPTS_PREFIX: &str = "claria-prompts/";

pub const SYSTEM_PROMPT: &str = "claria-prompts/system-prompt.md";

pub const EXTRACTION_PROMPT: &str = "claria-prompts/pdf-extraction.md";

/// Legacy key for the system prompt before the `claria-prompts/` migration.
/// Used as a read fallback so existing buckets keep working.
pub const LEGACY_SYSTEM_PROMPT: &str = "system-prompt.md";

pub const PROVISIONER_STATE: &str = "_state/provisioner.json";

pub const PREFERENCES: &str = "_state/preferences.json";
