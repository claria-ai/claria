use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};

use claria_bedrock::converse::StopSignal;
use claria_core::{
    model_id::CacheTtlChoice,
    models::chat_history::{ChatMessage, ChatRole},
};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use claria_desktop::config::{ClariaConfig, CredentialSource};
use claria_records::RecordCache;

/// An `SdkConfig` cached with the inputs it was built from. The SDK's pooled
/// HTTP connector lives inside the `SdkConfig`, so reusing it across commands
/// keeps connections warm instead of paying DNS/TCP/TLS setup per call.
/// The S3 client built from that config rides along and is invalidated with
/// it, so commands never rebuild a client per call.
pub struct CachedSdkConfig {
    pub region: String,
    pub credentials: CredentialSource,
    pub sdk_config: aws_config::SdkConfig,
    pub s3: aws_sdk_s3::Client,
}

pub(crate) struct PendingReportTemplate {
    pub(crate) client_id: uuid::Uuid,
    pub(crate) writer_template_id: uuid::Uuid,
    pub(crate) writer_template_name: String,
    pub(crate) source_docx: Vec<u8>,
    pub(crate) imported: claria_docx::ImportedTemplate,
}

const CHAT_PROMPT_CACHE_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatPromptCacheState {
    Cold,
    Reusable,
    Stale,
    PrefixChanged,
}

impl ChatPromptCacheState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Reusable => "reusable",
            Self::Stale => "stale",
            Self::PrefixChanged => "prefix_changed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChatPromptCacheKey {
    chat_id: uuid::Uuid,
    model_id: String,
}

struct CachedChatPrefix {
    system_digest: [u8; 32],
    message_digest: [u8; 32],
    message_count: usize,
    refreshed_at: Instant,
    /// The tier the provider entry was written at. Stored rather than
    /// assumed, so an entry written at one hour is not declared stale on
    /// the five-minute schedule.
    ttl: CacheTtlChoice,
}

/// Process-local knowledge of provider-side five-minute chat cache entries.
/// Only hashes and counts are retained here—never chat or record text.
pub(crate) struct ChatPromptCache {
    inner: StdMutex<lru::LruCache<ChatPromptCacheKey, CachedChatPrefix>>,
}

impl ChatPromptCache {
    fn new() -> Self {
        Self {
            inner: StdMutex::new(lru::LruCache::new(
                NonZeroUsize::new(CHAT_PROMPT_CACHE_CAPACITY).expect("nonzero cache capacity"),
            )),
        }
    }

    pub(crate) fn classify(
        &self,
        chat_id: uuid::Uuid,
        model_id: &str,
        system_prompt: &str,
        messages: &[ChatMessage],
    ) -> ChatPromptCacheState {
        self.classify_at(chat_id, model_id, system_prompt, messages, Instant::now())
    }

    fn classify_at(
        &self,
        chat_id: uuid::Uuid,
        model_id: &str,
        system_prompt: &str,
        messages: &[ChatMessage],
        now: Instant,
    ) -> ChatPromptCacheState {
        let key = ChatPromptCacheKey {
            chat_id,
            model_id: model_id.to_string(),
        };
        let mut cache = self.inner.lock().expect("chat prompt cache lock poisoned");
        let Some(cached) = cache.get(&key) else {
            return ChatPromptCacheState::Cold;
        };
        if now.duration_since(cached.refreshed_at) >= cached.ttl.window() {
            cache.pop(&key);
            return ChatPromptCacheState::Stale;
        }
        if cached.system_digest != digest_bytes(system_prompt.as_bytes())
            || messages.len() < cached.message_count
            || cached.message_digest != digest_messages(&messages[..cached.message_count])
        {
            return ChatPromptCacheState::PrefixChanged;
        }
        ChatPromptCacheState::Reusable
    }

    pub(crate) fn refresh(
        &self,
        chat_id: uuid::Uuid,
        model_id: &str,
        system_prompt: &str,
        messages: &[ChatMessage],
        ttl: CacheTtlChoice,
    ) {
        self.refresh_at(
            chat_id,
            model_id,
            system_prompt,
            messages,
            ttl,
            Instant::now(),
        );
    }

    fn refresh_at(
        &self,
        chat_id: uuid::Uuid,
        model_id: &str,
        system_prompt: &str,
        messages: &[ChatMessage],
        ttl: CacheTtlChoice,
        now: Instant,
    ) {
        self.inner
            .lock()
            .expect("chat prompt cache lock poisoned")
            .put(
                ChatPromptCacheKey {
                    chat_id,
                    model_id: model_id.to_string(),
                },
                CachedChatPrefix {
                    system_digest: digest_bytes(system_prompt.as_bytes()),
                    message_digest: digest_messages(messages),
                    message_count: messages.len(),
                    refreshed_at: now,
                    ttl,
                },
            );
    }
}

fn digest_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn digest_messages(messages: &[ChatMessage]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for message in messages {
        digest.update(match message.role {
            ChatRole::User => [0_u8],
            ChatRole::Assistant => [1_u8],
        });
        digest.update((message.content.len() as u64).to_be_bytes());
        digest.update(message.content.as_bytes());
    }
    digest.finalize().into()
}

pub struct DesktopState {
    pub config: Arc<Mutex<Option<ClariaConfig>>>,
    pub sdk_config: Arc<Mutex<Option<CachedSdkConfig>>>,
    pub local_transcriber: Arc<std::sync::Mutex<claria_transcribe::LocalTranscriber>>,
    pub record_cache: Arc<RecordCache>,
    /// Per-version report-revision summaries; version IDs are immutable so
    /// entries never go stale.
    pub revision_cache: Arc<claria_report_store::RevisionCache>,
    /// Exact transient Writer protocol, retained only for Bedrock's default
    /// five-minute prompt-cache window.
    pub report_prompt_cache: Arc<claria::ReportPromptCache>,
    /// Hash-only state for deciding whether a reloaded client chat still has
    /// a reusable provider cache prefix.
    pub(crate) chat_prompt_cache: Arc<ChatPromptCache>,
    /// Parsed managed-template candidates. Validated source bytes stay only
    /// long enough to write the immutable formatting snapshot; local paths are
    /// never retained.
    pub(crate) pending_report_templates: Arc<Mutex<HashMap<uuid::Uuid, PendingReportTemplate>>>,
    /// Stop signals for streamed work that is still in flight — chat turns,
    /// writer turns, and whole-report drafting runs alike — keyed by the
    /// stream id the frontend minted before it invoked the command. Entries
    /// are registered for the length of one call and removed on every exit
    /// path, so a stale Stop press finds nothing and does nothing.
    pub(crate) stream_stops: Arc<StdMutex<HashMap<uuid::Uuid, StopSignal>>>,
    /// Temporary assumed-role credentials, keyed by the opaque handle the
    /// `assume_role` command returned. The secrets never cross the IPC
    /// boundary — the frontend only ever holds the handle — and they expire
    /// with the STS session, so entries are replaced rather than accumulated.
    pub(crate) assumed_role_credentials: Arc<Mutex<HashMap<uuid::Uuid, CredentialSource>>>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(None)),
            sdk_config: Arc::new(Mutex::new(None)),
            local_transcriber: Arc::new(std::sync::Mutex::new(
                claria_transcribe::LocalTranscriber::default(),
            )),
            record_cache: Arc::new(RecordCache::new()),
            revision_cache: Arc::new(claria_report_store::RevisionCache::new()),
            report_prompt_cache: Arc::new(claria::ReportPromptCache::new()),
            chat_prompt_cache: Arc::new(ChatPromptCache::new()),
            pending_report_templates: Arc::new(Mutex::new(HashMap::new())),
            stream_stops: Arc::new(StdMutex::new(HashMap::new())),
            assumed_role_credentials: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn message(role: ChatRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: content.to_string(),
        }
    }

    #[test]
    fn reloaded_chat_prefix_is_reusable_until_the_five_minute_window_closes() {
        let cache = ChatPromptCache::new();
        let chat_id = uuid::Uuid::new_v4();
        let model_id = "us.anthropic.claude-sonnet-4-6";
        let system = "stable record context";
        let first_request = vec![message(ChatRole::User, "First question")];
        let refreshed_at = Instant::now();
        cache.refresh_at(
            chat_id,
            model_id,
            system,
            &first_request,
            CacheTtlChoice::FiveMinutes,
            refreshed_at,
        );

        let resumed_request = vec![
            first_request[0].clone(),
            message(ChatRole::Assistant, "First answer"),
            message(ChatRole::User, "Follow-up"),
        ];
        assert_eq!(
            cache.classify_at(
                chat_id,
                model_id,
                system,
                &resumed_request,
                refreshed_at + Duration::from_secs(2 * 60),
            ),
            ChatPromptCacheState::Reusable
        );
        assert_eq!(
            cache.classify_at(
                chat_id,
                model_id,
                system,
                &resumed_request,
                refreshed_at + Duration::from_secs(5 * 60),
            ),
            ChatPromptCacheState::Stale
        );
    }

    /// The mirror expires on the tier the entry was written at. A one-hour
    /// entry declared stale after five minutes would report a miss on every
    /// resumed conversation the extended tier exists to serve.
    #[test]
    fn a_one_hour_prefix_survives_the_five_minute_mark() {
        let cache = ChatPromptCache::new();
        let chat_id = uuid::Uuid::new_v4();
        let model_id = "us.anthropic.claude-sonnet-4-6";
        let system = "stable record context";
        let request = vec![message(ChatRole::User, "First question")];
        let refreshed_at = Instant::now();
        cache.refresh_at(
            chat_id,
            model_id,
            system,
            &request,
            CacheTtlChoice::OneHour,
            refreshed_at,
        );

        assert_eq!(
            cache.classify_at(
                chat_id,
                model_id,
                system,
                &request,
                refreshed_at + Duration::from_secs(20 * 60),
            ),
            ChatPromptCacheState::Reusable
        );
        assert_eq!(
            cache.classify_at(
                chat_id,
                model_id,
                system,
                &request,
                refreshed_at + Duration::from_secs(60 * 60),
            ),
            ChatPromptCacheState::Stale
        );
    }

    #[test]
    fn changed_context_is_not_mistaken_for_a_reusable_prefix() {
        let cache = ChatPromptCache::new();
        let chat_id = uuid::Uuid::new_v4();
        let model_id = "us.anthropic.claude-sonnet-4-6";
        let request = vec![message(ChatRole::User, "Question")];
        let now = Instant::now();
        cache.refresh_at(
            chat_id,
            model_id,
            "old context",
            &request,
            CacheTtlChoice::FiveMinutes,
            now,
        );

        assert_eq!(
            cache.classify_at(
                chat_id,
                model_id,
                "new context",
                &request,
                now + Duration::from_secs(60),
            ),
            ChatPromptCacheState::PrefixChanged
        );
    }
}
