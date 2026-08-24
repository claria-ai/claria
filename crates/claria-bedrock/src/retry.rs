//! Backoff for Bedrock calls the service asked the caller to repeat.
//!
//! One owner for "which failures are worth sending again, and how long to
//! wait", so a flow that fans requests out cannot invent its own schedule and
//! quietly hammer a throttled account.

use backon::{ExponentialBuilder, Retryable};

use crate::error::BedrockError;

/// Delay before the first retry. Bedrock throttling clears on the order of a
/// second, and the schedule doubles from here.
const BASE_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// Total attempts, the first included: 1s, 2s, 4s of jittered waiting before
/// the caller sees the failure. Long enough to ride out a burst, short
/// enough that a genuinely unavailable model still fails inside a turn.
///
/// Public because a caller that reports retries to a reader has to say what
/// it is counting towards, and "attempt 2 of 4" must not be a second copy of
/// this number.
pub const MAX_ATTEMPTS: u32 = 4;

/// Wall-clock ceiling on the whole retry sequence for one call.
///
/// [`MAX_ATTEMPTS`] alone stopped bounding anything useful once the stream
/// waits grew. An attempt ends when one of two waits runs out, so the
/// pathological attempt costs the first-frame wait plus the idle wait —
/// twenty minutes at the defaults, which are now the same ten-minute pair
/// for the writer and the analysis family alike, and forty at the ceiling a
/// clinician may type. Four of those in a row is eighty minutes of a turn
/// that has already failed the same way three times, which is not a wait
/// anyone would choose over an error message.
///
/// So the schedule is bounded by elapsed time as well as by count: once a
/// call has spent this long, the failure it just took is the one the caller
/// gets. The check happens before a retry is scheduled and never
/// mid-attempt, because an attempt that is still streaming is working and
/// must not be killed for taking time — which is the whole point of the
/// longer waits. The true worst case is therefore this budget plus one
/// full attempt, not this budget.
///
/// Twenty minutes because it tracks that pathological attempt exactly. A
/// budget below it would be spent by a single attempt, and a call whose
/// first attempt has already exhausted the budget is retried zero times —
/// which is the opposite of what the long waits are for. So this number
/// moves whenever the defaults it is derived from move. Throttle retries
/// are unaffected: they fail in milliseconds and their whole schedule fits
/// in seven seconds.
pub const MAX_RETRY_ELAPSED: std::time::Duration = std::time::Duration::from_secs(1200);

/// Told about each scheduled retry: the number of the attempt about to be
/// made — 2 for the first retry, up to [`MAX_ATTEMPTS`] — and how long the
/// wrapper will wait before making it.
///
/// `Send + Sync` because the retried future is spawned onto the runtime by
/// callers that fan requests out.
pub type RetryObserver<'a> = &'a (dyn Fn(u32, std::time::Duration) + Send + Sync);

/// Jittered exponential backoff for retryable Bedrock failures.
///
/// Jitter matters as soon as more than one request is in flight: a fan-out
/// that backs off in lockstep re-throttles itself on every wave.
fn backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(BASE_DELAY)
        .with_factor(2.0)
        .with_jitter()
        .with_max_times(MAX_ATTEMPTS as usize - 1)
}

/// Run `operation`, retrying only failures that the request can survive
/// being repeated: a throttle or capacity signal
/// ([`BedrockError::is_retryable_throttle`]), or a call that provably never
/// completed ([`BedrockError::is_interrupted_before_completion`]).
///
/// Everything else — a schema violation, a truncated response, an exhausted
/// quota — returns on the first attempt, because sending the same bytes
/// again would fail the same way.
///
/// Retryable or not, no attempt is scheduled once the sequence has run for
/// [`MAX_RETRY_ELAPSED`]; see there for why a count alone is not a bound.
///
/// `operation` is rebuilt per attempt, so the caller keeps ownership of
/// whatever the request borrows. `label` names the call in the retry log
/// line; it is a static operation name, never request content.
pub async fn with_throttle_retry<F, Fut, T>(
    label: &'static str,
    operation: F,
) -> Result<T, BedrockError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, BedrockError>>,
{
    with_throttle_retry_observed(label, None, operation).await
}

/// [`with_throttle_retry`] with a caller-supplied observer.
///
/// The retry is otherwise identical — same predicate, same schedule, same
/// log line. What this adds is a way for a flow that shows progress to say
/// so: a retried call is invisible from the outside, because the request is
/// re-sent under the same turn and the reader only sees the wait.
///
/// `on_retry` is called once per scheduled retry, before the delay, with the
/// number of the attempt about to be made and the wait ahead of it. It runs
/// on the retrying task, so it must not block.
pub async fn with_throttle_retry_observed<F, Fut, T>(
    label: &'static str,
    on_retry: Option<RetryObserver<'_>>,
    operation: F,
) -> Result<T, BedrockError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, BedrockError>>,
{
    let mut attempts_made = 1_u32;
    // `tokio::time::Instant` rather than `std::time::Instant` so a paused
    // test clock advances this budget the same way it advances the backoff.
    let started = tokio::time::Instant::now();
    operation
        .retry(backoff())
        .when(|error: &BedrockError| {
            if !(error.is_retryable_throttle() || error.is_interrupted_before_completion()) {
                return false;
            }
            let elapsed = started.elapsed();
            if elapsed >= MAX_RETRY_ELAPSED {
                tracing::warn!(
                    operation = label,
                    ?elapsed,
                    budget = ?MAX_RETRY_ELAPSED,
                    "Bedrock call is out of retry budget, surfacing the failure: {error}"
                );
                return false;
            }
            true
        })
        .notify(|error: &BedrockError, delay| {
            attempts_made = attempts_made.saturating_add(1);
            let attempt = attempts_made;
            tracing::warn!(
                operation = label,
                attempt,
                max_attempts = MAX_ATTEMPTS,
                ?delay,
                "Bedrock call will be retried: {error}"
            );
            if let Some(on_retry) = on_retry {
                on_retry(attempt, delay);
            }
        })
        .await
}
