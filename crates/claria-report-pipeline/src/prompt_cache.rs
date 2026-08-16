//! Process-local transient protocol cache aligned to the provider's
//! prompt-cache window.

use std::{
    num::NonZeroUsize,
    sync::Mutex,
    time::{Duration, Instant},
};

use claria_core::models::report::ReportProtocolMessage;
use uuid::Uuid;

const REPORT_PROMPT_CACHE_CAPACITY: usize = 8;
const REPORT_PROMPT_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
struct CachedReportProtocol {
    model_id: String,
    turn_count: usize,
    protocol: Vec<ReportProtocolMessage>,
    refreshed_at: Instant,
}

/// Small process-local LRU that keeps the exact (unsanitized) Bedrock
/// protocol only for the provider's five-minute cache window. This lets a
/// Writing session survive a frontend remount without forfeiting its prompt
/// cache while keeping record/tool content out of persisted history.
pub struct ReportPromptCache {
    inner: Mutex<lru::LruCache<Uuid, CachedReportProtocol>>,
}

impl Default for ReportPromptCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportPromptCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(lru::LruCache::new(
                NonZeroUsize::new(REPORT_PROMPT_CACHE_CAPACITY).expect("nonzero cache capacity"),
            )),
        }
    }

    pub(crate) fn reusable_protocol(
        &self,
        report_id: Uuid,
        model_id: &str,
        turn_count: usize,
    ) -> Option<Vec<ReportProtocolMessage>> {
        let now = Instant::now();
        let mut cache = self
            .inner
            .lock()
            .expect("report prompt cache lock poisoned");
        let cached = cache.get(&report_id).cloned()?;
        if now.duration_since(cached.refreshed_at) >= REPORT_PROMPT_CACHE_TTL {
            cache.pop(&report_id);
            tracing::debug!(%report_id, "report prompt cache is stale");
            return None;
        }
        if cached.model_id != model_id || cached.turn_count != turn_count {
            cache.pop(&report_id);
            tracing::debug!(%report_id, "report prompt cache prefix changed");
            return None;
        }
        tracing::debug!(%report_id, "reusing active report prompt cache");
        Some(cached.protocol)
    }

    pub(crate) fn refresh(
        &self,
        report_id: Uuid,
        model_id: &str,
        turn_count: usize,
        protocol: Vec<ReportProtocolMessage>,
    ) {
        self.inner
            .lock()
            .expect("report prompt cache lock poisoned")
            .put(
                report_id,
                CachedReportProtocol {
                    model_id: model_id.to_string(),
                    turn_count,
                    protocol,
                    refreshed_at: Instant::now(),
                },
            );
    }

    pub fn invalidate(&self, report_id: Uuid) {
        self.inner
            .lock()
            .expect("report prompt cache lock poisoned")
            .pop(&report_id);
    }
}
