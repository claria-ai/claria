use claria_report_pipeline::{
    DEFAULT_MAX_CONVERSE_CALLS, DEFAULT_MAX_RETAINED_TURNS, DEFAULT_MAX_TOOL_ROUNDS,
    DEFAULT_MAX_TOOL_USES_PER_RESPONSE, MAX_CONFIGURABLE_CONVERSE_CALLS,
    MAX_CONFIGURABLE_RETAINED_TURNS, MAX_CONFIGURABLE_TOOL_ROUNDS,
    MAX_CONFIGURABLE_TOOL_USES_PER_RESPONSE, ReportTurnLimits,
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
