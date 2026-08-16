//! Which Bedrock failures are worth sending again, and what the retry
//! wrapper does with them. Driven by synthetic errors: the classification is
//! the contract, and it must not need a live service to pin down.

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use claria_bedrock::{
    error::BedrockError,
    retry::{MAX_ATTEMPTS, with_throttle_retry, with_throttle_retry_observed},
};

fn service(code: &str, status: Option<u16>) -> BedrockError {
    BedrockError::Service {
        operation: "report ConverseStream",
        status,
        code: code.to_string(),
        request_id: None,
        message: "synthetic".to_string(),
    }
}

#[test]
fn throttle_and_capacity_signals_are_retryable() {
    for code in [
        "ThrottlingException",
        "ServiceUnavailableException",
        "ModelNotReadyException",
    ] {
        assert!(service(code, None).is_retryable_throttle(), "{code}");
    }
    // Status alone is enough when the code is unfamiliar.
    assert!(service("SomethingNew", Some(429)).is_retryable_throttle());
    assert!(service("SomethingNew", Some(503)).is_retryable_throttle());
}

/// An account quota does not clear on a backoff timer. Retrying it spends
/// the user's time and buries the one error whose fix is a quota increase.
#[test]
fn quota_exhaustion_and_request_faults_stay_terminal() {
    assert!(!service("ServiceQuotaExceededException", Some(400)).is_retryable_throttle());
    assert!(!service("ValidationException", Some(400)).is_retryable_throttle());
    assert!(!service("AccessDeniedException", Some(403)).is_retryable_throttle());
    assert!(
        !BedrockError::ResponseTruncated {
            max_output_tokens: 32_768
        }
        .is_retryable_throttle()
    );
    assert!(!BedrockError::ContextWindowExceeded.is_retryable_throttle());
}

/// Throttling and interruption are separate predicates that the wrapper
/// treats alike, so neither can start being retried without the other.
#[test]
fn interrupted_calls_are_not_reported_as_throttles() {
    let interrupted = BedrockError::StreamInterrupted {
        operation: "report ConverseStream",
        message: "synthetic".to_string(),
    };
    assert!(interrupted.is_interrupted_before_completion());
    assert!(!interrupted.is_retryable_throttle());
}

#[tokio::test(start_paused = true)]
async fn a_throttled_call_is_retried_until_it_succeeds() {
    let attempts = AtomicUsize::new(0);
    let outcome = with_throttle_retry("report ConverseStream", || async {
        if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
            return Err(service("ThrottlingException", Some(429)));
        }
        Ok(7)
    })
    .await
    .expect("retried call succeeds");
    assert_eq!(outcome, 7);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn a_terminal_failure_is_not_sent_again() {
    let attempts = AtomicUsize::new(0);
    let error = with_throttle_retry("report ConverseStream", || async {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err::<(), _>(service("ValidationException", Some(400)))
    })
    .await
    .expect_err("terminal failure surfaces");
    assert!(matches!(error, BedrockError::Service { code, .. } if code == "ValidationException"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

/// The attempt ceiling is real: a service that never recovers must give the
/// caller its error back rather than retrying inside the turn forever.
#[tokio::test(start_paused = true)]
async fn retries_are_bounded() {
    let attempts = AtomicUsize::new(0);
    let error = with_throttle_retry("report ConverseStream", || async {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err::<(), _>(service("ThrottlingException", Some(429)))
    })
    .await
    .expect_err("exhausted retries surface the last failure");
    assert!(error.is_retryable_throttle());
    assert_eq!(attempts.load(Ordering::SeqCst), MAX_ATTEMPTS as usize);
}

/// A retried call is invisible from the outside: the request goes out again
/// under the same turn, and the reader sees only the wait. The observer is
/// how a flow that shows progress gets to say what is happening, so it has
/// to be told which attempt is coming and how long the wait is.
#[tokio::test(start_paused = true)]
async fn the_retry_observer_is_told_which_attempt_is_coming() {
    let seen: Mutex<Vec<(u32, std::time::Duration)>> = Mutex::new(Vec::new());
    let observe = |attempt: u32, delay: std::time::Duration| {
        seen.lock().expect("observer lock").push((attempt, delay));
    };
    let attempts = AtomicUsize::new(0);

    let outcome = with_throttle_retry_observed("report_plan", Some(&observe), || async {
        if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
            return Err(service("ThrottlingException", Some(429)));
        }
        Ok(7)
    })
    .await
    .expect("retried call succeeds");

    assert_eq!(outcome, 7);
    let seen = seen.into_inner().expect("observer lock");
    // Numbered by the attempt about to be made, so the first retry is
    // attempt 2 — the number a reader is shown alongside the ceiling.
    assert_eq!(
        seen.iter().map(|(attempt, _)| *attempt).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert!(
        seen.iter()
            .all(|(_, delay)| *delay >= std::time::Duration::from_millis(1)),
        "the observer was handed no wait to report: {seen:?}"
    );
}

/// Exhaustion is where the count matters most: the last thing the observer
/// reports has to be the last attempt the wrapper will make.
#[tokio::test(start_paused = true)]
async fn the_retry_observer_stops_at_the_attempt_ceiling() {
    let seen: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    let observe = |attempt: u32, _delay: std::time::Duration| {
        seen.lock().expect("observer lock").push(attempt);
    };

    with_throttle_retry_observed("report_plan", Some(&observe), || async {
        Err::<(), _>(service("ThrottlingException", Some(429)))
    })
    .await
    .expect_err("exhausted retries surface the last failure");

    assert_eq!(seen.into_inner().expect("observer lock"), vec![2, 3, 4]);
    assert_eq!(MAX_ATTEMPTS, 4);
}
