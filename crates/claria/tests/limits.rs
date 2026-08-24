use claria::{
    BedrockRuntimeLimits, DEFAULT_MAX_CONVERSE_CALLS, DEFAULT_MAX_RETAINED_TURNS,
    DEFAULT_MAX_TOOL_ROUNDS, DEFAULT_MAX_TOOL_USES_PER_RESPONSE, MAX_CONFIGURABLE_CONVERSE_CALLS,
    MAX_CONFIGURABLE_RETAINED_TURNS, MAX_CONFIGURABLE_TIMEOUT_SECS, MAX_CONFIGURABLE_TOOL_ROUNDS,
    MAX_CONFIGURABLE_TOOL_USES_PER_RESPONSE, MAX_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS,
    MIN_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS, ReportTurnLimits,
};

#[test]
fn writer_limit_defaults_are_ten_times_the_original_guardrails() {
    let limits = ReportTurnLimits::default();

    assert_eq!(limits.max_tool_rounds(), 40);
    assert_eq!(limits.max_converse_calls(), 50);
    assert_eq!(limits.max_tool_uses_per_response(), 80);
    assert_eq!(limits.max_retained_turns(), 200);
    assert_eq!(limits.max_tool_rounds(), DEFAULT_MAX_TOOL_ROUNDS);
    assert_eq!(limits.max_converse_calls(), DEFAULT_MAX_CONVERSE_CALLS);
    assert_eq!(
        limits.max_tool_uses_per_response(),
        DEFAULT_MAX_TOOL_USES_PER_RESPONSE
    );
    assert_eq!(limits.max_retained_turns(), DEFAULT_MAX_RETAINED_TURNS);
}

#[test]
fn writer_limits_enforce_structural_safety_ceilings() {
    assert!(
        ReportTurnLimits::try_new(
            MAX_CONFIGURABLE_TOOL_ROUNDS,
            MAX_CONFIGURABLE_CONVERSE_CALLS,
            MAX_CONFIGURABLE_TOOL_USES_PER_RESPONSE,
            MAX_CONFIGURABLE_RETAINED_TURNS,
        )
        .is_ok()
    );
    assert!(ReportTurnLimits::try_new(4, 1, 1, 1).is_ok());
    assert!(ReportTurnLimits::try_new(0, 2, 1, 1).is_err());
    assert!(ReportTurnLimits::try_new(4, 0, 1, 1).is_err());
    assert!(
        ReportTurnLimits::try_new(1, 2, MAX_CONFIGURABLE_TOOL_USES_PER_RESPONSE + 1, 1,).is_err()
    );
    assert!(ReportTurnLimits::try_new(1, 2, 1, MAX_CONFIGURABLE_RETAINED_TURNS + 1).is_err());
}

/// The four waits are one number: ten minutes, before a first frame and
/// between frames, for the writer and for the planner and reviewer alike.
/// They were graded once, on the argument that a stream which has produced
/// nothing is free to abandon — but the retry that argument buys re-reads
/// the same input from cold, and it is the demanding requests, the ones
/// whose progress is worth the most, that take longest to answer.
#[test]
fn bedrock_runtime_defaults_wait_ten_minutes_everywhere() {
    let runtime = BedrockRuntimeLimits::default();

    assert_eq!(runtime.writer_first_frame_timeout_secs, 600);
    assert_eq!(runtime.writer_idle_timeout_secs, 600);
    assert_eq!(runtime.writer_max_output_tokens, 32_768);
    assert_eq!(runtime.analysis_first_frame_timeout_secs, 600);
    assert_eq!(runtime.analysis_idle_timeout_secs, 600);
    assert_eq!(ReportTurnLimits::default().runtime(), runtime);

    // Every default stays under the ceiling with room above it, so the
    // settings can still be raised and are not merely lowerable.
    for wait in [
        runtime.writer_first_frame_timeout_secs,
        runtime.writer_idle_timeout_secs,
        runtime.analysis_first_frame_timeout_secs,
        runtime.analysis_idle_timeout_secs,
    ] {
        assert!(
            wait < MAX_CONFIGURABLE_TIMEOUT_SECS,
            "a default equal to the ceiling leaves the setting nowhere to go: \
             {wait} vs {MAX_CONFIGURABLE_TIMEOUT_SECS}"
        );
    }
}

#[test]
fn bedrock_runtime_rejects_values_that_would_fail_every_call() {
    let ok = BedrockRuntimeLimits::default();
    assert!(ok.validate().is_ok());

    // A zero wait abandons the request before the service can answer it.
    for zeroed in [
        BedrockRuntimeLimits {
            writer_first_frame_timeout_secs: 0,
            ..ok
        },
        BedrockRuntimeLimits {
            writer_idle_timeout_secs: 0,
            ..ok
        },
        BedrockRuntimeLimits {
            analysis_first_frame_timeout_secs: 0,
            ..ok
        },
        BedrockRuntimeLimits {
            analysis_idle_timeout_secs: 0,
            ..ok
        },
    ] {
        assert!(zeroed.validate().is_err());
    }

    // A wait past the ceiling would let a dead socket hang a turn.
    assert!(
        BedrockRuntimeLimits {
            writer_first_frame_timeout_secs: MAX_CONFIGURABLE_TIMEOUT_SECS + 1,
            ..ok
        }
        .validate()
        .is_err()
    );
    assert!(
        BedrockRuntimeLimits {
            writer_first_frame_timeout_secs: MAX_CONFIGURABLE_TIMEOUT_SECS,
            ..ok
        }
        .validate()
        .is_ok()
    );

    // An output ceiling outside what a Claude model will produce.
    assert!(
        BedrockRuntimeLimits {
            writer_max_output_tokens: MIN_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS - 1,
            ..ok
        }
        .validate()
        .is_err()
    );
    assert!(
        BedrockRuntimeLimits {
            writer_max_output_tokens: MAX_CONFIGURABLE_WRITER_MAX_OUTPUT_TOKENS + 1,
            ..ok
        }
        .validate()
        .is_err()
    );
}

#[test]
fn raised_waits_reach_the_bounds_one_call_is_made_under() {
    let limits = ReportTurnLimits::default()
        .with_runtime(BedrockRuntimeLimits {
            writer_first_frame_timeout_secs: 480,
            writer_idle_timeout_secs: 240,
            analysis_first_frame_timeout_secs: 500,
            analysis_idle_timeout_secs: 300,
            ..BedrockRuntimeLimits::default()
        })
        .expect("within range");

    assert_eq!(limits.stream_bounds().first_frame_secs(), 480);
    assert_eq!(limits.stream_bounds().idle_secs(), 240);
    assert_eq!(
        limits.runtime().analysis_stream_bounds().first_frame_secs(),
        500
    );
    assert_eq!(limits.runtime().analysis_stream_bounds().idle_secs(), 300);
    assert_eq!(limits.writer_first_frame_timeout_secs(), 480);
}

#[test]
fn a_rejected_runtime_leaves_the_limits_untouched() {
    let limits = ReportTurnLimits::default();
    let rejected = limits.with_runtime(BedrockRuntimeLimits {
        writer_idle_timeout_secs: 0,
        ..BedrockRuntimeLimits::default()
    });

    assert!(rejected.is_err());
    assert_eq!(limits.runtime(), BedrockRuntimeLimits::default());
}

#[test]
fn scaling_a_plan_does_not_disturb_the_runtime_dials() {
    let raised = ReportTurnLimits::default()
        .with_runtime(BedrockRuntimeLimits {
            writer_first_frame_timeout_secs: 300,
            ..BedrockRuntimeLimits::default()
        })
        .expect("within range");

    assert_eq!(
        raised.scaled_for_plan(45).runtime(),
        raised.runtime(),
        "raising a plan's call ceiling must not reset the clinician's waits"
    );
}
