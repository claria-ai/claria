//! Integration test for Bedrock model availability classification.
//!
//! Calls real AWS APIs and requires valid credentials in the environment.
//!
//! Run with:
//! `cargo test -p claria-provisioner --test bedrock_model_agreement_test -- --ignored --nocapture`

use claria_provisioner::{build_manifest, build_syncers};

/// Diagnostic: classify every matching Claude model and print the syncer's
/// read state. Models AWS hasn't enabled for the account (gradual rollout)
/// must land in `blocked_models` with a reason pointing at AWS Support, even
/// when `GetFoundationModelAvailability` reports them fully available.
#[tokio::test]
#[ignore]
async fn diagnostic_classify_models() {
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;

    let manifest = build_manifest("000000000000", "claria", "us-west-2");
    let syncers = build_syncers(&config, &manifest, None);
    let syncer = syncers
        .iter()
        .find(|s| s.spec().resource_type == "bedrock_model_agreement")
        .expect("manifest should contain a bedrock_model_agreement spec");

    let state = syncer
        .read()
        .await
        .expect("read should succeed")
        .expect("read should return state");

    println!(
        "{}",
        serde_json::to_string_pretty(&state).expect("state serializes")
    );

    let invokable = state["invokable_models"]
        .as_array()
        .expect("invokable_models is an array");
    assert!(
        !invokable.is_empty(),
        "expected at least one invokable Claude model"
    );

    // Every blocked model must carry a human-readable reason.
    for blocked in state["blocked_models"]
        .as_array()
        .expect("blocked_models is an array")
    {
        assert!(
            !blocked["reason"]
                .as_str()
                .expect("reason is a string")
                .is_empty(),
            "blocked model {} has an empty reason",
            blocked["model_id"]
        );
    }
}
