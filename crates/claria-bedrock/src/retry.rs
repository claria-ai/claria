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
    operation
        .retry(backoff())
        .when(|error: &BedrockError| {
            error.is_retryable_throttle() || error.is_interrupted_before_completion()
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
