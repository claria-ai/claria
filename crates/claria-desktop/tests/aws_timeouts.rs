//! The explicit AWS transport policy, and its inheritance by the service
//! clients built from it.
//!
//! The SDK's own defaults are 3.1s connect and nothing else — no read,
//! operation, or operation-attempt timeout at all. These tests pin the
//! deliberate policy in place of that so a future SDK bump, or a dropped
//! builder call, cannot quietly change how long Claria waits on AWS.

use std::time::Duration;

use claria_desktop::{
    aws::{
        CONNECT_TIMEOUT, READ_TIMEOUT, STALLED_STREAM_GRACE_PERIOD, build_aws_config,
        stalled_stream_protection, timeout_config,
    },
    config::CredentialSource,
};

fn test_credentials() -> CredentialSource {
    CredentialSource::Inline {
        access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
        secret_access_key: "secret".to_string(),
        session_token: None,
    }
}

#[test]
fn connecting_fails_fast() {
    let config = timeout_config();
    assert_eq!(config.connect_timeout(), Some(CONNECT_TIMEOUT));
    assert!(
        CONNECT_TIMEOUT <= Duration::from_secs(10),
        "an unreachable endpoint should fail in seconds"
    );
}

/// The read timeout looks like a responsiveness knob and is not one: it
/// bounds the wait for response headers, and a unary Converse sends no
/// headers until the model has finished generating. Tightening it to
/// something that feels snappy would abort a large text extraction, which
/// is the same failure mode this release exists to remove.
#[test]
fn the_read_timeout_clears_the_slowest_legitimate_generation() {
    let config = timeout_config();
    assert_eq!(config.read_timeout(), Some(READ_TIMEOUT));
    assert!(
        READ_TIMEOUT >= Duration::from_secs(10 * 60),
        "a read timeout this short aborts a long unary Converse mid-generation"
    );
}

/// The one that matters most: a total-duration cap would kill a Bedrock
/// stream writing a long clinical section, which is the failure this whole
/// change exists to prevent. Read and connect timeouts bound the phases
/// where nothing is happening; neither bounds generation.
#[test]
fn no_total_duration_cap_is_imposed_on_a_call() {
    let config = timeout_config();
    assert_eq!(
        config.operation_timeout(),
        None,
        "an operation timeout caps total call duration and would abort a long generation"
    );
    assert_eq!(
        config.operation_attempt_timeout(),
        None,
        "an attempt timeout caps a single try and would abort a long generation"
    );
}

/// Supplying a `StalledStreamProtectionConfig` at all means owning its
/// grace period: the builder defaults to 20 seconds while the runtime
/// plugin this replaces uses 5, so leaving it unset silently loosens the
/// protection rather than preserving it.
#[test]
fn stalled_stream_protection_is_enabled_with_an_explicit_grace_period() {
    let config = stalled_stream_protection();
    assert!(config.is_enabled());
    assert!(config.upload_enabled());
    assert!(config.download_enabled());
    assert_eq!(config.grace_period(), STALLED_STREAM_GRACE_PERIOD);
    assert_ne!(
        STALLED_STREAM_GRACE_PERIOD,
        Duration::from_secs(20),
        "20s is the builder's unset default — the value here must be a choice"
    );
}

#[tokio::test]
async fn the_built_sdk_config_carries_the_policy() {
    let sdk_config = build_aws_config("us-east-1", &test_credentials()).await;

    let timeouts = sdk_config.timeout_config().expect("timeout config");
    assert_eq!(timeouts.connect_timeout(), Some(CONNECT_TIMEOUT));
    assert_eq!(timeouts.read_timeout(), Some(READ_TIMEOUT));
    assert_eq!(timeouts.operation_timeout(), None);
    assert_eq!(timeouts.operation_attempt_timeout(), None);

    let stalled = sdk_config
        .stalled_stream_protection()
        .expect("stalled stream protection");
    assert!(stalled.is_enabled());
    assert_eq!(stalled.grace_period(), STALLED_STREAM_GRACE_PERIOD);
}

/// One policy covers Bedrock and S3, so the S3 client factory must not drop
/// it on the way through. It also must not diverge: the shared HTTP client
/// caches connectors by connect/read timeout, so per-service values would
/// split the pool the app keeps warm on purpose.
#[tokio::test]
async fn service_clients_inherit_the_policy() {
    let sdk_config = build_aws_config("us-east-1", &test_credentials()).await;

    let s3 = claria_storage::client::from_config(&sdk_config);
    let s3_timeouts = s3.config().timeout_config().expect("s3 timeout config");
    assert_eq!(s3_timeouts.connect_timeout(), Some(CONNECT_TIMEOUT));
    assert_eq!(s3_timeouts.read_timeout(), Some(READ_TIMEOUT));
    assert_eq!(s3_timeouts.operation_timeout(), None);
    assert_eq!(
        s3.config()
            .stalled_stream_protection()
            .expect("s3 stalled stream protection")
            .grace_period(),
        STALLED_STREAM_GRACE_PERIOD
    );
}
