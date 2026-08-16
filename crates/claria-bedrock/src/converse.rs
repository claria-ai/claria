//! Shared Bedrock Converse plumbing for every claria-bedrock flow.
//!
//! One owner for runtime-client construction, structured service-error
//! classification, response-text collection, optional usage extraction, and
//! the `CountTokens` wrapper. Chat, extraction, translation, and the report
//! writer all speak to Bedrock through this module so their error and usage
//! semantics cannot drift apart.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use aws_sdk_bedrockruntime::{
    error::{DisplayErrorContext, ProvideErrorMetadata, SdkError},
    operation::RequestId,
    types::{
        CachePointBlock, CachePointType, CacheTtl, ContentBlock, ContentBlockDelta,
        ConverseStreamOutput, ConverseTokensRequest, CountTokensInput, Message, StopReason,
        TokenUsage,
    },
};
use aws_smithy_types::{Document, Number};
use claria_core::{model_id::CacheTtlChoice, models::turn_usage::TurnUsage};

use crate::{error::BedrockError, tokens};

/// Build the Bedrock Runtime client every Converse flow uses.
pub(crate) fn runtime_client(config: &aws_config::SdkConfig) -> aws_sdk_bedrockruntime::Client {
    aws_sdk_bedrockruntime::Client::new(config)
}

/// Classify any Bedrock SDK failure into the one structured error shape.
///
/// Service errors preserve HTTP status, service code, request ID, and the
/// service message. Non-service failures (connect/timeout/deserialization)
/// keep their full [`DisplayErrorContext`] chain instead of collapsing to
/// "unhandled error".
pub(crate) fn classify_error<E>(operation: &'static str, error: SdkError<E>) -> BedrockError
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    let status = error
        .raw_response()
        .map(|response| response.status().as_u16());
    let (code, request_id, message) = match error.as_service_error() {
        Some(service_error) => (
            service_error
                .meta()
                .code()
                .unwrap_or("UnknownServiceError")
                .to_string(),
            service_error.meta().request_id().map(ToString::to_string),
            service_error
                .meta()
                .message()
                .unwrap_or("the service returned no error message")
                .to_string(),
        ),
        None => (
            "DispatchFailure".to_string(),
            None,
            DisplayErrorContext(&error).to_string(),
        ),
    };
    tracing::error!(operation, ?status, code, ?request_id, "Bedrock call failed");
    BedrockError::Service {
        operation,
        status,
        code,
        request_id,
        message,
    }
}

/// Map a domain TTL choice to the wire TTL for a cache point.
///
/// `FiveMinutes` maps to `None` — the server default — so requests that use
/// the default TTL stay byte-identical to the pre-TTL wire shape.
pub(crate) fn sdk_cache_ttl(choice: CacheTtlChoice) -> Option<CacheTtl> {
    match choice {
        CacheTtlChoice::FiveMinutes => None,
        CacheTtlChoice::OneHour => Some(CacheTtl::OneHour),
    }
}

/// Build one prompt-cache `cachePoint` block, with an optional extended
/// TTL. `None` omits the ttl field entirely (server default: 5 minutes).
pub(crate) fn cache_point(ttl: Option<CacheTtl>) -> Result<CachePointBlock, BedrockError> {
    let mut builder = CachePointBlock::builder().r#type(CachePointType::Default);
    if let Some(ttl) = ttl {
        builder = builder.ttl(ttl);
    }
    builder
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))
}

/// Append a cache point to the final message's content so the next request
/// in the conversation reads everything up to (and including) this message
/// from cache. Shared by the chat and report flows; an empty conversation
/// is returned unchanged.
pub(crate) fn with_tail_cache_point(
    mut messages: Vec<Message>,
    ttl: Option<CacheTtl>,
) -> Result<Vec<Message>, BedrockError> {
    let Some(last) = messages.pop() else {
        return Ok(messages);
    };
    let mut content = last.content().to_vec();
    content.push(ContentBlock::CachePoint(cache_point(ttl)?));
    let rebuilt = Message::builder()
        .role(last.role().clone())
        .set_content(Some(content))
        .build()
        .map_err(|error| BedrockError::Invocation(error.to_string()))?;
    messages.push(rebuilt);
    Ok(messages)
}

/// Caller-decided placement of the `cachePoint` blocks on one report-family
/// request.
///
/// Bedrock caches a *prefix*, so where the markers go decides what survives
/// the next call — and the service rejects mixed TTLs in one request, so a
/// single [`Self::ttl`] covers every point the plan emits. The wire level
/// owns none of that judgement: a plan says where the points go, and
/// [`crate::report`] puts them exactly there.
///
/// Placement is expressed in three places because that is where Bedrock will
/// take a marker: after the system blocks, after a chosen content block of a
/// chosen message, and at the conversation tail.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachePlan {
    /// Tier every cache point in this plan is written at. `FiveMinutes` maps
    /// to no `ttl` field on the wire — see [`sdk_cache_ttl`].
    pub ttl: CacheTtlChoice,
    /// Emit a `cachePoint` as the last system block, so the system policy
    /// (and the tool schemas rendered ahead of it) read from cache.
    pub after_system: bool,
    /// Emit a `cachePoint` at the end of the last message, so the next call
    /// in the loop reads the whole conversation so far from cache.
    pub tail: bool,
    /// `(message index, block index)` coordinates, zero-based, after each of
    /// which a `cachePoint` is inserted. Both indices address the request's
    /// protocol messages as the caller passed them, before any cache point
    /// was added.
    pub after_blocks: Vec<(usize, usize)>,
}

impl CachePlan {
    /// Bedrock's ceiling on `cachePoint` blocks in one request. A request
    /// over it is rejected outright, so the plan is validated at
    /// construction rather than discovered at the wire.
    pub const MAX_CACHE_POINTS: usize = 4;

    /// A plan that emits nothing, producing the exact request shape a
    /// caching-incapable model gets.
    pub fn disabled() -> Self {
        Self {
            ttl: CacheTtlChoice::FiveMinutes,
            after_system: false,
            tail: false,
            after_blocks: Vec::new(),
        }
    }

    /// Build a plan, rejecting one that asks for more cache points than
    /// Bedrock accepts.
    pub fn new(
        ttl: CacheTtlChoice,
        after_system: bool,
        tail: bool,
        after_blocks: Vec<(usize, usize)>,
    ) -> Result<Self, BedrockError> {
        let plan = Self {
            ttl,
            after_system,
            tail,
            after_blocks,
        };
        let points = plan.point_count();
        if points > Self::MAX_CACHE_POINTS {
            return Err(BedrockError::SchemaViolation(format!(
                "cache plan asks for {points} cache points; Bedrock accepts at most {}",
                Self::MAX_CACHE_POINTS
            )));
        }
        Ok(plan)
    }

    /// The placement the report family has always used: one point after the
    /// system policy and one at the conversation tail, at the default
    /// five-minute tier. The writer's loop re-reads within minutes, so the
    /// doubled one-hour write rate would never pay off.
    ///
    /// Gated on the model family alone, reproducing today's behaviour: the
    /// writer has never consulted the desktop's `prompt_caching_enabled`
    /// flag, so this passes it as on.
    pub fn report_default(capabilities: claria_core::model_id::ModelCapabilities) -> Self {
        Self {
            ttl: CacheTtlChoice::FiveMinutes,
            after_system: true,
            tail: true,
            after_blocks: Vec::new(),
        }
        .gated(capabilities, true)
    }

    /// The whole-report drafting conversation's placement: no point on the
    /// system policy, one after each caller-named checkpoint in the opening
    /// message, and a moving tail point.
    ///
    /// The tier is the extended one-hour window wherever the family accepts
    /// it. A drafting run writes a large frozen prefix once and then reads it
    /// across every section it writes, so the doubled write rate is paid on
    /// one call and recovered on the rest — the opposite trade from a
    /// targeted edit, which re-reads within minutes and takes the cheaper
    /// five-minute write. Bedrock rejects mixed TTLs in one request, so the
    /// tier necessarily covers the tail point too.
    pub fn full_draft(
        capabilities: claria_core::model_id::ModelCapabilities,
        after_blocks: Vec<(usize, usize)>,
    ) -> Result<Self, BedrockError> {
        Ok(
            Self::new(CacheTtlChoice::OneHour, false, true, after_blocks)?
                .gated(capabilities, true),
        )
    }

    /// The analysis family's placement: one point after the last system
    /// block, nothing else.
    ///
    /// The planner and the review passes put the record corpus and the
    /// template structure in the system blocks, above that point, and vary
    /// only the messages below it. Bedrock invalidates in tool → system →
    /// message order, so every analysis request on a given model reads the
    /// same corpus prefix from cache no matter which tool it forces or which
    /// property it asks about. The extended tier is the right trade for the
    /// same reason the drafting run takes it: one write, many reads, spread
    /// over a session rather than seconds.
    pub fn analysis(capabilities: claria_core::model_id::ModelCapabilities) -> Self {
        Self {
            ttl: CacheTtlChoice::OneHour,
            after_system: true,
            tail: false,
            after_blocks: Vec::new(),
        }
        .gated(capabilities, true)
    }

    /// Gate a plan on what the model can do and what the user asked for:
    /// either saying no yields [`Self::disabled`], and a one-hour TTL the
    /// family does not accept is downgraded rather than sent and rejected.
    pub fn gated(
        self,
        capabilities: claria_core::model_id::ModelCapabilities,
        prompt_caching_enabled: bool,
    ) -> Self {
        if !prompt_caching_enabled || !capabilities.prompt_caching {
            return Self::disabled();
        }
        let ttl = match self.ttl {
            CacheTtlChoice::OneHour if !capabilities.supports_extended_cache_ttl => {
                CacheTtlChoice::FiveMinutes
            }
            ttl => ttl,
        };
        Self { ttl, ..self }
    }

    /// How many `cachePoint` blocks this plan puts on the wire.
    pub fn point_count(&self) -> usize {
        usize::from(self.after_system) + usize::from(self.tail) + self.after_blocks.len()
    }

    /// Whether the plan emits any cache point at all.
    pub fn is_enabled(&self) -> bool {
        self.point_count() > 0
    }

    /// The TTL to record on the turn's usage: `Some` only when the request
    /// actually carried cache points, so an uncached turn prices as uncached
    /// instead of claiming a tier it never wrote at.
    pub fn effective_ttl(&self) -> Option<CacheTtlChoice> {
        self.is_enabled().then_some(self.ttl)
    }
}

/// Caching off unless a caller asks for it, so a default-constructed plan
/// cannot silently start writing cache entries.
impl Default for CachePlan {
    fn default() -> Self {
        Self::disabled()
    }
}

/// Insert a `cachePoint` immediately after each `(message, block)`
/// coordinate, both zero-based against `messages` as passed in.
///
/// Insertions run back to front so every coordinate keeps addressing the
/// block the caller meant; a coordinate that does not exist is an error
/// rather than a silently skipped cache point, because a plan that misses is
/// a plan that quietly stops caching.
pub(crate) fn with_cache_points_after(
    mut messages: Vec<Message>,
    after_blocks: &[(usize, usize)],
    ttl: Option<CacheTtl>,
) -> Result<Vec<Message>, BedrockError> {
    if after_blocks.is_empty() {
        return Ok(messages);
    }
    let mut coordinates = after_blocks.to_vec();
    coordinates.sort_unstable();
    coordinates.dedup();
    for &(message_index, block_index) in coordinates.iter().rev() {
        let message = messages.get(message_index).ok_or_else(|| {
            BedrockError::SchemaViolation(format!(
                "cache plan names message {message_index}, but the request has {} messages",
                messages.len()
            ))
        })?;
        let mut content = message.content().to_vec();
        if block_index >= content.len() {
            return Err(BedrockError::SchemaViolation(format!(
                "cache plan names block {block_index} of message {message_index}, which has {} blocks",
                content.len()
            )));
        }
        content.insert(
            block_index + 1,
            ContentBlock::CachePoint(cache_point(ttl.clone())?),
        );
        messages[message_index] = Message::builder()
            .role(message.role().clone())
            .set_content(Some(content))
            .build()
            .map_err(|error| BedrockError::Invocation(error.to_string()))?;
    }
    Ok(messages)
}

/// Concatenate the text blocks of an assistant message.
pub(crate) fn collect_text(message: &Message) -> String {
    message
        .content()
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Extract per-turn usage, preserving absence.
///
/// The AWS JSON deserializer may materialize a default all-zero usage
/// structure when the service omits the block. A successful Converse call
/// necessarily consumes tokens, so an all-zero shape is reported as `None`
/// rather than a misleading metered zero — callers render absence.
pub(crate) fn optional_usage(
    usage: Option<&TokenUsage>,
    model_id: &str,
    cache_ttl: Option<CacheTtlChoice>,
) -> Option<TurnUsage> {
    usage.and_then(|usage| {
        let extracted = tokens::extract_turn_usage(usage, model_id, cache_ttl);
        (extracted.input_tokens > 0
            || extracted.output_tokens > 0
            || extracted.cache_read_input_tokens > 0
            || extracted.cache_write_input_tokens > 0)
            .then_some(extracted)
    })
}

/// Enforce a complete text response.
///
/// `EndTurn` and `StopSequence` are complete; `MaxTokens` and
/// `ModelContextWindowExceeded` become typed errors so no caller can
/// persist silently truncated output as success. Any other stop reason is a
/// protocol surprise for a text-only flow.
pub(crate) fn ensure_complete_text_response(
    stop_reason: &StopReason,
    max_output_tokens: u32,
) -> Result<(), BedrockError> {
    match stop_reason {
        StopReason::EndTurn | StopReason::StopSequence => Ok(()),
        StopReason::MaxTokens => Err(BedrockError::ResponseTruncated { max_output_tokens }),
        StopReason::ModelContextWindowExceeded => Err(BedrockError::ContextWindowExceeded),
        other => Err(BedrockError::ResponseParse(format!(
            "unexpected stop reason {other:?} for a text-only response"
        ))),
    }
}

/// The completed result of a streamed Converse call.
#[derive(Debug)]
pub struct StreamOutcome {
    /// The full accumulated assistant text.
    pub text: String,
    /// The wire stop reason (e.g. `end_turn`), for the caller's terminal
    /// stream event. `max_tokens` reaches the caller as a value rather than
    /// an error — see [`StreamCollector::finish`] — so a truncated answer
    /// can be kept and labelled. Every other incomplete reason still fails.
    pub stop_reason: String,
    /// Per-turn usage from the trailing `metadata` event; `None` preserved
    /// when the service omitted it (see [`optional_usage`]).
    pub usage: Option<TurnUsage>,
    /// Wall-clock time of the full streamed exchange, stamped by the caller
    /// that drove the stream; `None` when not measured (synthetic tests).
    pub latency_ms: Option<u64>,
}

/// Longest wait for a `ConverseStream` to start answering before the
/// request is treated as one the service never began.
///
/// Sized for the longest prefill a live call really has: a cross-region
/// profile can miss the prompt cache and re-prefill the whole input cold,
/// and a field failure showed that wait exceeding two minutes when the two
/// waits below shared a single five-minute bound. Ninety seconds and a
/// fresh request beat waiting out a five-minute silence, and a request that
/// produced nothing is safe to send again verbatim.
pub(crate) const STREAM_FIRST_FRAME_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(90);

/// Longest silence tolerated between two frames of a response stream that
/// has already started.
///
/// The AWS SDK's stalled-stream protection does not cover this: the
/// generated `ConverseStream` operation registers no
/// `StalledStreamProtectionInterceptor`, unlike every unary operation. With
/// no read timeout on the body either — the SDK's read timeout bounds only
/// the wait for response headers — a `ConverseStream` whose socket dies
/// mid-generation would otherwise hang forever.
///
/// Frames flow continuously once generation starts — the gaps are the
/// pauses between reasoning deltas, not minutes — so a silent minute
/// mid-response means the connection is gone, not that the model is busy.
pub(crate) const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// First-frame wait for the analysis family, which sends the largest input
/// of any flow — the whole record corpus, in system blocks — and gets no
/// frame until the model has read all of it.
pub(crate) const ANALYSIS_STREAM_FIRST_FRAME_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(120);

/// Inter-frame wait for the analysis family. One forced tool call emits a
/// single large structured document, and the model deliberates inside it
/// rather than between sentences, so the pauses that count as normal are
/// longer than a chat reply's.
pub(crate) const ANALYSIS_STREAM_IDLE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(90);

/// How long one call's stream may stay quiet, before it starts and after.
///
/// Carried per call rather than read from a global const, because the two
/// waits that are generous for a forced-tool analysis request are a hang
/// for a chat reply. Every value is a compile-time family default — there
/// is no preference knob and no per-request tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamBounds {
    /// Longest wait for the first event frame; exceeding it means the
    /// service never began the response.
    pub(crate) first_frame: std::time::Duration,
    /// Longest silence between two frames of a stream already under way.
    pub(crate) idle: std::time::Duration,
}

impl StreamBounds {
    /// Chat and writer turns: text arrives continuously once generation
    /// starts, so a silent minute is a dead socket.
    pub(crate) const fn conversational() -> Self {
        Self {
            first_frame: STREAM_FIRST_FRAME_TIMEOUT,
            idle: STREAM_IDLE_TIMEOUT,
        }
    }

    /// Forced-tool analysis calls: the biggest input and the longest
    /// single structured answer of any family.
    pub(crate) const fn analysis() -> Self {
        Self {
            first_frame: ANALYSIS_STREAM_FIRST_FRAME_TIMEOUT,
            idle: ANALYSIS_STREAM_IDLE_TIMEOUT,
        }
    }
}

/// Stop reason recorded for a turn the reader ended from the UI. Not a
/// Bedrock value — the service never got to send one — so it is spelled
/// differently from every wire reason and callers can label it as a choice
/// rather than a failure.
pub const STOPPED_BY_USER: &str = "stopped_by_user";

/// Cooperative stop for an in-flight streamed Converse call.
///
/// Cloneable and cheap: the caller keeps one copy to fire from a Stop button
/// and hands another to the stream loop. A default signal is one nobody can
/// fire, which is what non-interactive callers want.
///
/// The stream loop awaits [`Self::stopped`] alongside the next frame rather
/// than checking between frames — a stream can sit silent for minutes while
/// the model prefills, and a Stop button that waits for the next token is
/// not a Stop button.
#[derive(Debug, Clone, Default)]
pub struct StopSignal {
    inner: Arc<StopState>,
}

#[derive(Debug, Default)]
struct StopState {
    stopped: AtomicBool,
    wake: tokio::sync::Notify,
}

impl StopSignal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the stream loop to stop. Idempotent, and safe to call after the
    /// turn has already finished.
    pub fn stop(&self) {
        self.inner.stopped.store(true, Ordering::Release);
        self.inner.wake.notify_waiters();
    }

    pub fn is_stopped(&self) -> bool {
        self.inner.stopped.load(Ordering::Acquire)
    }

    /// Whether two handles name the same signal. Lets a registry of live
    /// turns drop its own entry without disturbing a newer turn that reused
    /// the key.
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Resolve once [`Self::stop`] has been called; pends forever otherwise.
    ///
    /// The waiter is registered before the flag is read, so a `stop` racing
    /// this call wakes it instead of being missed.
    pub async fn stopped(&self) {
        loop {
            let mut wake = std::pin::pin!(self.inner.wake.notified());
            wake.as_mut().enable();
            if self.is_stopped() {
                return;
            }
            wake.await;
        }
    }
}

/// Send a `ConverseStream` request and wait for the service to start
/// answering, bounded by [`StreamBounds::first_frame`].
///
/// The first-frame wait belongs here rather than in the stream loop,
/// because that is where the SDK spends it: the generated `send` resolves
/// only once the first event frame has arrived, since it has to look at
/// that frame to decide whether it is an `initial-response`. Nothing else
/// bounds the wait — the read timeout is satisfied by the response headers,
/// which arrive long before the first token — so a request the service
/// accepted and never began otherwise hangs for as long as the socket
/// stays open.
///
/// A request that produced no frame is one the model never started, so the
/// failure says that instead of claiming a connection lost mid-response,
/// and it is [`BedrockError::StreamInterrupted`] like the mid-stream case:
/// nothing was generated, so re-sending it is safe.
pub(crate) async fn start_converse_stream<T, E>(
    operation: &'static str,
    bounds: StreamBounds,
    send: impl std::future::Future<Output = Result<T, SdkError<E>>>,
) -> Result<T, BedrockError>
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    match tokio::time::timeout(bounds.first_frame, send).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(classify_error(operation, error)),
        Err(_elapsed) => {
            let seconds = bounds.first_frame.as_secs();
            tracing::error!(operation, seconds, "Bedrock never started responding");
            Err(BedrockError::StreamInterrupted {
                operation,
                message: format!(
                    "{operation} never started responding within {seconds} seconds and was \
                     abandoned. The request was queued or the connection was lost before the \
                     first token; nothing was generated."
                ),
            })
        }
    }
}

/// Receive the next frame of a Converse response stream, bounded by
/// [`StreamBounds::idle`]. `Ok(None)` ends the stream.
///
/// Shared by the chat, analysis, and writer stream loops so their mid-stream
/// error shape cannot drift; the bound itself is the caller's, because what
/// counts as a silence differs per request family. Mid-stream failures carry
/// a raw event frame rather than an HTTP response, so the full
/// [`DisplayErrorContext`] chain is preserved instead of collapsing to
/// "unhandled error".
pub(crate) async fn recv_stream_event(
    operation: &'static str,
    bounds: StreamBounds,
    stream: &mut aws_sdk_bedrockruntime::primitives::event_stream::EventReceiver<
        ConverseStreamOutput,
        aws_sdk_bedrockruntime::types::error::ConverseStreamOutputError,
    >,
) -> Result<Option<ConverseStreamOutput>, BedrockError> {
    match tokio::time::timeout(bounds.idle, stream.recv()).await {
        Ok(Ok(event)) => Ok(event),
        Ok(Err(error)) => {
            tracing::error!(operation, "Bedrock stream failed");
            Err(BedrockError::StreamInterrupted {
                operation,
                message: format!(
                    "{operation} failed before the response completed: {}",
                    DisplayErrorContext(&error)
                ),
            })
        }
        Err(_elapsed) => {
            let seconds = bounds.idle.as_secs();
            tracing::error!(operation, seconds, "Bedrock stream went silent");
            Err(BedrockError::StreamInterrupted {
                operation,
                message: format!(
                    "{operation} stopped sending data for {seconds} seconds and was abandoned. \
                     The connection to Bedrock was lost mid-response; the request was not completed."
                ),
            })
        }
    }
}

/// Accumulates a `ConverseStream` response: text deltas, the stop reason
/// from `messageStop`, and usage from the trailing `metadata` event.
///
/// Pure event-folding, so tests can drive it with synthetic events; the
/// terminal [`Self::finish`] applies the same truncation and stop-reason
/// rules as the unary path.
#[derive(Debug, Default)]
pub struct StreamCollector {
    text: String,
    stop_reason: Option<StopReason>,
    usage: Option<TokenUsage>,
}

impl StreamCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorb one stream event, returning the text delta it carried (if any)
    /// so the caller can forward it incrementally.
    pub fn absorb(&mut self, event: ConverseStreamOutput) -> Option<String> {
        match event {
            ConverseStreamOutput::ContentBlockDelta(event) => match event.delta {
                Some(ContentBlockDelta::Text(text)) => {
                    self.text.push_str(&text);
                    Some(text)
                }
                _ => None,
            },
            ConverseStreamOutput::MessageStop(event) => {
                self.stop_reason = Some(event.stop_reason);
                None
            }
            ConverseStreamOutput::Metadata(event) => {
                self.usage = event.usage;
                None
            }
            _ => None,
        }
    }

    /// Close out the stream: a missing `messageStop` is a protocol error,
    /// context overflow becomes the same typed error as the unary path, and
    /// an omitted usage block stays `None`. `cache_ttl` is the TTL the
    /// request's cache points carried (`None` when uncached), recorded on
    /// the usage so cache writes price at the right tier.
    ///
    /// Truncation is the one incomplete reason that survives: the text has
    /// already been streamed to the reader, so discarding it deletes an
    /// answer they watched arrive. It returns as a `max_tokens` stop reason
    /// for the caller to label. Context overflow has no partial worth
    /// keeping, and the unary path is unchanged.
    pub fn finish(
        self,
        model_id: &str,
        max_output_tokens: u32,
        cache_ttl: Option<CacheTtlChoice>,
    ) -> Result<StreamOutcome, BedrockError> {
        let stop_reason = self.stop_reason.ok_or_else(|| {
            BedrockError::ResponseParse(
                "the response stream ended without a messageStop event".to_string(),
            )
        })?;
        if !matches!(stop_reason, StopReason::MaxTokens) {
            ensure_complete_text_response(&stop_reason, max_output_tokens)?;
        }
        let usage = optional_usage(self.usage.as_ref(), model_id, cache_ttl);
        Ok(StreamOutcome {
            text: self.text,
            stop_reason: stop_reason.as_str().to_string(),
            usage,
            latency_ms: None,
        })
    }

    /// Close out a stream the reader stopped from the UI.
    ///
    /// Never an error: a missing `messageStop` is exactly what stopping
    /// means, and the text that did arrive is the answer the reader chose to
    /// keep. Usage is normally `None` — the trailing `metadata` frame comes
    /// after the stop reason, so an abandoned stream is unmetered even
    /// though Bedrock billed the tokens it had already produced.
    pub fn finish_stopped(
        self,
        model_id: &str,
        cache_ttl: Option<CacheTtlChoice>,
    ) -> StreamOutcome {
        StreamOutcome {
            text: self.text,
            stop_reason: STOPPED_BY_USER.to_string(),
            usage: optional_usage(self.usage.as_ref(), model_id, cache_ttl),
            latency_ms: None,
        }
    }
}

/// Opt-in per-request model tuning. The desktop gates every knob against
/// the central capability table before constructing this — this layer
/// applies exactly what it is given and adds nothing by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelTuning {
    /// Send `thinking: {"type": "adaptive"}` via
    /// `additionalModelRequestFields` (Claude 4.6+).
    pub adaptive_thinking: bool,
    /// Send `output_config: {"effort": ...}` via
    /// `additionalModelRequestFields` (Claude 4.5+).
    pub effort: Option<EffortLevel>,
    /// Send `inferenceConfig.temperature` (rejected by Claude 4.7-class and
    /// newer models — the capability table gates it to generations that
    /// accept it).
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    Max,
}

impl EffortLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

/// Anthropic-specific request fields that ride outside the Converse
/// `inferenceConfig`, as a Smithy document; `None` when the tuning requests
/// nothing so untuned requests stay byte-identical to the pre-tuning shape.
pub(crate) fn additional_request_fields(
    tuning: ModelTuning,
) -> Result<Option<Document>, BedrockError> {
    let mut fields = serde_json::Map::new();
    if tuning.adaptive_thinking {
        fields.insert(
            "thinking".to_string(),
            serde_json::json!({"type": "adaptive"}),
        );
    }
    if let Some(effort) = tuning.effort {
        fields.insert(
            "output_config".to_string(),
            serde_json::json!({"effort": effort.as_str()}),
        );
    }
    if fields.is_empty() {
        return Ok(None);
    }
    json_to_document(&serde_json::Value::Object(fields)).map(Some)
}

/// Structured observability line for the token budget a request runs under,
/// emitted once per chat request and once per writer turn.
///
/// `context_window_tokens` is what the central capability table resolved for
/// this model ID, which for any inference profile without an explicit
/// `:48k` / `:200k` / `:1m` suffix is an assumption. Recording it beside the
/// budget derived from it is what makes "we are budgeting against a fraction
/// of the real window" answerable from a console export instead of guessed
/// at. Fields are model IDs and counts — never prompt or record content.
pub(crate) fn log_model_budget(
    operation: &'static str,
    model_id: &str,
    input_budget_tokens: u32,
    output_reserve_tokens: u32,
) {
    tracing::info!(
        target: "claria_bedrock::budget",
        operation,
        model_id,
        context_window_tokens =
            claria_core::model_id::ModelCapabilities::for_id(model_id).context_window_tokens,
        input_budget_tokens,
        output_reserve_tokens,
        "token budget resolved"
    );
}

/// Structured cache/usage observability line for one completed Converse
/// call, shared by the chat and report flows.
///
/// `operation` names the request family rather than the SDK call — `chat`,
/// `report_targeted`, `report_full_draft` — because cache behaviour differs
/// per family and a console export that lumps them together answers nothing.
/// It is a fixed label, never derived from user input. `hit_rate` is the fraction of
/// input tokens served from cache; `cache_ttl` names the write tier, and
/// `stop_reason`, `latency_ms`, and `max_tokens` let a console export answer
/// "was it truncated, how long did it take, at which ceiling?". Fields are
/// model IDs, counts, rates, and durations — never prompt or response
/// content.
pub(crate) fn log_turn_usage(
    operation: &'static str,
    model_id: &str,
    usage: Option<&TurnUsage>,
    stop_reason: Option<&str>,
    latency_ms: Option<u64>,
    max_tokens: u32,
) {
    if let Some(usage) = usage {
        let total_input =
            usage.input_tokens + usage.cache_read_input_tokens + usage.cache_write_input_tokens;
        let hit_rate = if total_input > 0 {
            usage.cache_read_input_tokens as f64 / total_input as f64
        } else {
            0.0
        };
        tracing::info!(
            target: "claria_bedrock::cache",
            operation,
            model_id,
            input_tokens = usage.input_tokens,
            cache_read = usage.cache_read_input_tokens,
            cache_write = usage.cache_write_input_tokens,
            cache_ttl = usage
                .cache_ttl
                .map_or("none", claria_core::model_id::CacheTtlChoice::as_str),
            hit_rate,
            cost_usd = usage.cost_usd,
            stop_reason = stop_reason.unwrap_or("unknown"),
            latency_ms,
            max_tokens,
            "turn complete"
        );
    } else {
        tracing::warn!(
            operation,
            model_id,
            stop_reason = stop_reason.unwrap_or("unknown"),
            latency_ms,
            max_tokens,
            "turn completed without a usage block"
        );
    }
}

/// Incremental input-token budget shared by the chat and report flows.
///
/// Token counts and character measures are kept together so a request is
/// counted with a real `CountTokens` call only when needed: appended content
/// is estimated at ~4 characters per token, and the estimate is re-verified
/// against the service only once it comes within ~10% of the budget.
pub(crate) struct InputTokenBudget {
    budget: u32,
    /// `(verified_tokens, chars_at_verification)` from the last real count.
    verified: Option<(u32, u64)>,
    /// Whether the first check must run a real count (report semantics:
    /// count once per turn) or may trust the character estimate until it
    /// nears the budget (chat semantics).
    verify_first: bool,
}

impl InputTokenBudget {
    /// A budget whose first check runs a real `CountTokens` call, then
    /// estimates increments.
    pub(crate) fn exact(budget: u32) -> Self {
        Self {
            budget,
            verified: None,
            verify_first: true,
        }
    }

    /// A budget that trusts the character estimate until it comes within
    /// ~10% of the budget, only then verifying with a real count.
    pub(crate) fn estimated(budget: u32) -> Self {
        Self {
            budget,
            verified: None,
            verify_first: false,
        }
    }

    /// Check a request of `current_chars` characters against the budget.
    ///
    /// `count` runs a real `CountTokens` for the current request shape;
    /// `over_budget` builds the caller's typed error from
    /// `(input_tokens, budget)`.
    pub(crate) async fn ensure_within<F, Fut, E>(
        &mut self,
        current_chars: u64,
        count: F,
        over_budget: E,
    ) -> Result<(), BedrockError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<u32, BedrockError>>,
        E: FnOnce(u32, u32) -> BedrockError,
    {
        let estimate = match self.verified {
            Some((tokens, chars)) => tokens.saturating_add(
                u32::try_from(current_chars.saturating_sub(chars) / 4).unwrap_or(u32::MAX),
            ),
            None if self.verify_first => u32::MAX,
            None => u32::try_from(current_chars / 4).unwrap_or(u32::MAX),
        };
        // Within 90% of the budget (or a mandatory first check): verify.
        if u64::from(estimate) * 10 < u64::from(self.budget) * 9 {
            return Ok(());
        }
        let tokens = count().await?;
        self.verified = Some((tokens, current_chars));
        if tokens > self.budget {
            Err(over_budget(tokens, self.budget))
        } else {
            Ok(())
        }
    }
}

/// Convert a `serde_json::Value` to the Smithy `Document` the Converse tool
/// APIs speak.
pub(crate) fn json_to_document(value: &serde_json::Value) -> Result<Document, BedrockError> {
    match value {
        serde_json::Value::Null => Ok(Document::Null),
        serde_json::Value::Bool(value) => Ok(Document::Bool(*value)),
        serde_json::Value::String(value) => Ok(Document::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .map(json_to_document)
            .collect::<Result<Vec<_>, _>>()
            .map(Document::Array),
        serde_json::Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), json_to_document(value)?)))
            .collect::<Result<HashMap<_, _>, BedrockError>>()
            .map(Document::Object),
        serde_json::Value::Number(value) => {
            let number = if let Some(value) = value.as_u64() {
                Number::PosInt(value)
            } else if let Some(value) = value.as_i64() {
                Number::NegInt(value)
            } else if let Some(value) = value.as_f64() {
                if !value.is_finite() {
                    return Err(BedrockError::Serialization(serde_json::Error::io(
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "non-finite JSON number",
                        ),
                    )));
                }
                Number::Float(value)
            } else {
                return Err(BedrockError::SchemaViolation(
                    "JSON number could not be represented for Bedrock".to_string(),
                ));
            };
            Ok(Document::Number(number))
        }
    }
}

/// Convert a Smithy `Document` (tool-call input) back into JSON.
pub(crate) fn document_to_json(document: &Document) -> Result<serde_json::Value, BedrockError> {
    match document {
        Document::Null => Ok(serde_json::Value::Null),
        Document::Bool(value) => Ok(serde_json::Value::Bool(*value)),
        Document::String(value) => Ok(serde_json::Value::String(value.clone())),
        Document::Array(values) => values
            .iter()
            .map(document_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        Document::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), document_to_json(value)?)))
            .collect::<Result<serde_json::Map<_, _>, BedrockError>>()
            .map(serde_json::Value::Object),
        Document::Number(Number::PosInt(value)) => Ok(serde_json::Value::Number((*value).into())),
        Document::Number(Number::NegInt(value)) => Ok(serde_json::Value::Number((*value).into())),
        Document::Number(Number::Float(value)) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                BedrockError::ResponseParse(
                    "Bedrock document contained a non-finite number".to_string(),
                )
            }),
    }
}

/// Count the input tokens of one Converse-shaped request against a bare
/// foundation model ID (`CountTokens` rejects inference-profile IDs).
pub(crate) async fn count_input_tokens(
    client: &aws_sdk_bedrockruntime::Client,
    model_id: &str,
    request: ConverseTokensRequest,
) -> Result<u32, BedrockError> {
    let response = client
        .count_tokens()
        .model_id(model_id)
        .input(CountTokensInput::Converse(request))
        .send()
        .await
        .map_err(|error| classify_error("CountTokens", error))?;
    Ok(u32::try_from(response.input_tokens()).unwrap_or(u32::MAX))
}
