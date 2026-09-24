//! The state document itself.
//!
//! `ProvisionerState.resources` is keyed by [`ResourceAddr`]. JSON object keys
//! are strings, so an address that serializes as a struct makes `serde_json`
//! reject the whole document — which meant no state holding any resource could
//! ever be written, and the record every teardown and orphan check reads from
//! stayed permanently empty.

use std::collections::HashMap;

use claria_provisioner::{
    ProvisionerState, ResourceAddr,
    state::{ResourceState, ResourceStatus, migrate_state_v1_to_v2},
};
use serde_json::json;

const BUCKET: &str = "123456789012-claria-data";

fn state_with(entries: &[(&str, &str)]) -> ProvisionerState {
    let mut resources = HashMap::new();
    for (resource_type, resource_name) in entries {
        let addr = ResourceAddr {
            resource_type: (*resource_type).into(),
            resource_name: (*resource_name).into(),
        };
        resources.insert(
            addr.clone(),
            ResourceState {
                resource_type: addr.resource_type.clone(),
                resource_id: addr.resource_name.clone(),
                status: ResourceStatus::Created,
                properties: json!({}),
            },
        );
    }
    let mut state = ProvisionerState::new("us-east-1".into(), BUCKET.into());
    state.resources = resources;
    state
}

/// The whole point: a state file with something in it has to be writable.
#[test]
fn a_populated_state_survives_a_round_trip() {
    let state = state_with(&[
        ("s3_bucket", BUCKET),
        ("cloudtrail_trail", "claria-trail"),
        ("iam_user", "claria-admin"),
    ]);

    let json = serde_json::to_vec_pretty(&state).expect("a non-empty state must serialize");
    let back: ProvisionerState = serde_json::from_slice(&json).expect("and load back");

    assert_eq!(back.resources.len(), 3);
    assert_eq!(back.region, state.region);
    for addr in state.resources.keys() {
        assert!(back.resources.contains_key(addr), "{addr} did not survive");
    }
}

/// The on-disk key is the documented `resource_type.resource_name` form, which
/// is also what the v1 migration writes.
#[test]
fn addresses_are_written_as_dotted_strings() {
    let state = state_with(&[("s3_bucket", BUCKET)]);
    let value = serde_json::to_value(&state).expect("serialize");

    let resources = value
        .get("resources")
        .and_then(|r| r.as_object())
        .expect("resources is an object");
    let keys: Vec<&String> = resources.keys().collect();
    assert_eq!(keys, vec![&format!("s3_bucket.{BUCKET}")]);
}

/// A resource name may itself contain dots, so only the first one separates.
#[test]
fn only_the_first_dot_separates_type_from_name() {
    let state = state_with(&[("bedrock_model_agreement", "anthropic.claude")]);
    let json = serde_json::to_vec(&state).expect("serialize");
    let back: ProvisionerState = serde_json::from_slice(&json).expect("load");

    let addr = back.resources.keys().next().expect("one resource");
    assert_eq!(addr.resource_type, "bedrock_model_agreement");
    assert_eq!(addr.resource_name, "anthropic.claude");
}

/// v1 kept resources under a bare type name. Refusing that key is what sends
/// `StatePersistence::load` down its migration path instead of accepting a
/// document it would misread.
#[test]
fn a_v1_document_is_refused_then_migrates() {
    let v1 = json!({
        "resources": {
            "s3_bucket": {
                "resource_type": "s3_bucket",
                "resource_id": BUCKET,
                "status": "created",
                "properties": {},
            }
        },
        "region": "us-east-1",
        "bucket": BUCKET,
    });

    serde_json::from_value::<ProvisionerState>(v1.clone())
        .expect_err("a bare type key is not an address");

    let migrated: ProvisionerState =
        serde_json::from_value(migrate_state_v1_to_v2(v1)).expect("migrated state loads");

    let addr = migrated.resources.keys().next().expect("one resource");
    assert_eq!(addr.resource_type, "s3_bucket");
    assert_eq!(addr.resource_name, BUCKET);
}
