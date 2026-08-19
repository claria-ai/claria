//! Which wait a failed writer call blames.
//!
//! A clinician sent to raise the wrong setting is worse off than one sent
//! nowhere: the two waits govern different failures, and the one that did
//! not run out will not help however high it goes.

use claria::{FIRST_FRAME_TIMEOUT_FIELD_LABEL, IDLE_TIMEOUT_FIELD_LABEL, interruption_advice};
use claria_bedrock::error::StreamInterruption;

const FIRST_FRAME_SECS: u32 = 90;
const IDLE_SECS: u32 = 60;

#[test]
fn a_response_that_started_and_stopped_blames_the_inter_chunk_wait() {
    let advice = interruption_advice(
        Some(StreamInterruption::WentSilent),
        FIRST_FRAME_SECS,
        IDLE_SECS,
    );

    assert!(advice.contains(IDLE_TIMEOUT_FIELD_LABEL), "{advice}");
    assert!(
        !advice.contains(FIRST_FRAME_TIMEOUT_FIELD_LABEL),
        "{advice}"
    );
    assert!(advice.contains("60 seconds"), "{advice}");
    assert!(
        !advice.contains("90 seconds"),
        "the wait that did not run out must not appear: {advice}"
    );
    // One enormous section is the other half of this failure, and the only
    // half a clinician can fix in their template.
    assert!(advice.contains("smaller sections"), "{advice}");
}

#[test]
fn a_response_that_never_started_blames_the_first_frame_wait() {
    let advice = interruption_advice(
        Some(StreamInterruption::NeverStarted),
        FIRST_FRAME_SECS,
        IDLE_SECS,
    );

    assert!(advice.contains(FIRST_FRAME_TIMEOUT_FIELD_LABEL), "{advice}");
    assert!(!advice.contains(IDLE_TIMEOUT_FIELD_LABEL), "{advice}");
    assert!(advice.contains("90 seconds"), "{advice}");
    assert!(!advice.contains("60 seconds"), "{advice}");
    assert!(advice.contains("cold prompt cache"), "{advice}");
}

/// A stream that failed outright is not a timeout, so no wait is named —
/// raising either one would be advice the failure does not support.
#[test]
fn a_dropped_stream_names_no_wait_at_all() {
    for interruption in [Some(StreamInterruption::Dropped), None] {
        let advice = interruption_advice(interruption, FIRST_FRAME_SECS, IDLE_SECS);

        assert!(
            !advice.contains(FIRST_FRAME_TIMEOUT_FIELD_LABEL),
            "{advice}"
        );
        assert!(!advice.contains(IDLE_TIMEOUT_FIELD_LABEL), "{advice}");
        assert!(advice.contains("Try again later"), "{advice}");
    }
}

/// The waits are read from the configuration, not from the constants they
/// default to: a clinician who already raised one has to be told the number
/// they are actually waiting.
#[test]
fn the_advice_quotes_the_configured_wait_not_the_default() {
    let advice = interruption_advice(Some(StreamInterruption::WentSilent), 420, 240);

    assert!(advice.contains("240 seconds"), "{advice}");
    assert!(!advice.contains("60 seconds"), "{advice}");
}
