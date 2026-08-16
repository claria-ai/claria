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
const MAX_ATTEMPTS: usize = 4;

/// Jittered exponential backoff for retryable Bedrock failures.
///
/// Jitter matters as soon as more than one request is in flight: a fan-out
/// that backs off in lockstep re-throttles itself on every wave.
fn backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(BASE_DELAY)
        .with_factor(2.0)
        .with_jitter()
        .with_max_times(MAX_ATTEMPTS - 1)
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
    operation
        .retry(backoff())
        .when(|error: &BedrockError| {
            error.is_retryable_throttle() || error.is_interrupted_before_completion()
        })
        .notify(|error: &BedrockError, delay| {
            tracing::warn!(
                operation = label,
                ?delay,
                "Bedrock call will be retried: {error}"
            );
        })
        .await
}
