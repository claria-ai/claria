//! The IAM policy has one desired state.
//!
//! `IamUserPolicySyncer` diffs the live policy against the manifest's
//! `iam_actions` and, when they differ, writes `claria_policy_document`. Those
//! were two hand-kept lists. A divergence in either direction — the comparison
//! is exact set equality — made the plan report a `Modify`, made applying it
//! report success, and made the next scan report the same drift, forever.
//!
//! The document is now rendered from the list the diff reads, and these tests
//! are what hold that true.

use std::collections::BTreeSet;

use claria_provisioner::{IamAction, IamScope, Manifest};
use serde_json::Value;

const ACCT: &str = "123456789012";
const SYS: &str = "claria";

fn policy() -> Value {
    serde_json::from_str(&claria_provisioner::claria_policy_document(SYS, ACCT))
        .expect("the policy document is JSON")
}

fn statements(policy: &Value) -> &Vec<Value> {
    policy["Statement"].as_array().expect("Statement is a list")
}

fn granted_actions(policy: &Value) -> BTreeSet<String> {
    statements(policy)
        .iter()
        .filter(|s| s["Effect"] == "Allow")
        .flat_map(|s| s["Action"].as_array().expect("Action is a list"))
        .map(|a| a.as_str().expect("an action is a string").to_string())
        .collect()
}

fn declared_actions() -> BTreeSet<String> {
    Manifest::iam_actions(ACCT, SYS)
        .into_iter()
        .map(|a| a.action)
        .collect()
}

/// The one invariant. Equality in both directions: an action in the policy but
/// not the manifest is power nothing asked for, and an action in the manifest
/// but not the policy is a drift that applying can never resolve.
#[test]
fn the_policy_grants_exactly_what_the_manifest_declares() {
    let granted = granted_actions(&policy());
    let declared = declared_actions();

    let unasked: Vec<_> = granted.difference(&declared).collect();
    assert!(
        unasked.is_empty(),
        "the policy grants actions no resource declares: {unasked:?}"
    );

    let ungranted: Vec<_> = declared.difference(&granted).collect();
    assert!(
        ungranted.is_empty(),
        "resources declare actions the policy does not grant: {ungranted:?}"
    );
}

/// The guard against the whole class: whatever the manifest says, the document
/// says. A test that hardcoded the list would be the third copy.
#[test]
fn an_action_added_to_the_manifest_reaches_the_policy() {
    let extra = IamAction::account("bedrock:ListCustomModels");
    let mut actions = Manifest::iam_actions(ACCT, SYS);
    assert!(
        !actions.contains(&extra),
        "pick an action the manifest does not already declare"
    );
    actions.push(extra.clone());

    let rendered: Value = serde_json::from_str(&claria_provisioner::render_policy_document(
        &actions, SYS, ACCT,
    ))
    .expect("JSON");

    assert!(granted_actions(&rendered).contains(&extra.action));
}

#[test]
fn every_action_lands_in_the_statement_its_scope_names() {
    let policy = policy();
    let by_sid = |sid: &str| -> BTreeSet<String> {
        statements(&policy)
            .iter()
            .filter(|s| s["Sid"] == sid)
            .flat_map(|s| s["Action"].as_array().expect("Action is a list"))
            .map(|a| a.as_str().expect("string").to_string())
            .collect()
    };

    for action in Manifest::iam_actions(ACCT, SYS) {
        let sid = match action.scope {
            IamScope::ClariaBuckets => "ClariaBuckets",
            IamScope::ClariaIdentity => "ClariaIdentity",
            IamScope::Account => "ClariaAccount",
        };
        assert!(
            by_sid(sid).contains(&action.action),
            "{} is scoped {:?} but is not in {sid}",
            action.action,
            action.scope
        );
    }
}

/// Security-scoping values fail closed. A bucket statement that reached
/// outside this deployment's prefix, or an identity statement that reached
/// past Claria's own user, would be the policy quietly widening itself.
#[test]
fn no_statement_reaches_past_what_its_scope_allows() {
    let policy = policy();
    for statement in statements(&policy) {
        let resources: Vec<String> = match &statement["Resource"] {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a
                .iter()
                .map(|r| r.as_str().expect("string").to_string())
                .collect(),
            other => panic!("unexpected Resource shape: {other}"),
        };

        match statement["Sid"]
            .as_str()
            .expect("every statement has a Sid")
        {
            "ClariaBuckets" => {
                for r in &resources {
                    assert!(
                        r.starts_with(&format!("arn:aws:s3:::{ACCT}-{SYS}-")),
                        "a bucket statement reaching outside this deployment: {r}"
                    );
                }
            }
            "ClariaIdentity" => {
                for r in &resources {
                    assert!(
                        r == &format!("arn:aws:iam::{ACCT}:user/claria-admin")
                            || r == &format!("arn:aws:iam::{ACCT}:policy/ClariaProvisionerAccess"),
                        "an identity statement reaching past Claria's own user: {r}"
                    );
                }
            }
            // Account-wide by necessity, so it stays narrow in actions
            // instead. The equality test above is what bounds it.
            "ClariaAccount" => assert_eq!(resources, vec!["*".to_string()]),
            other => panic!("unknown statement {other}"),
        }
    }
}

/// The account ID is a security-scoping value and must never be a wildcard.
#[test]
fn the_account_id_is_never_widened_into_a_wildcard() {
    let rendered = claria_provisioner::claria_policy_document(SYS, ACCT);
    assert!(rendered.contains(ACCT), "the account ID must be concrete");
    assert!(
        !rendered.contains("arn:aws:iam::*"),
        "an IAM ARN was widened to every account"
    );
    assert!(
        !rendered.contains("arn:aws:s3:::*"),
        "a bucket ARN was widened to every bucket"
    );
}

/// Several resources ask for the same action, and a policy whose action lists
/// reorder between renders reads as a change in every diff.
#[test]
fn the_rendered_policy_is_stable_and_free_of_duplicates() {
    let once = claria_provisioner::claria_policy_document(SYS, ACCT);
    let twice = claria_provisioner::claria_policy_document(SYS, ACCT);
    assert_eq!(once, twice);

    for statement in statements(&policy()) {
        let actions: Vec<&str> = statement["Action"]
            .as_array()
            .expect("Action is a list")
            .iter()
            .map(|a| a.as_str().expect("string"))
            .collect();
        let mut sorted = actions.clone();
        sorted.sort_unstable();
        assert_eq!(actions, sorted, "actions are not sorted");

        let unique: BTreeSet<&&str> = actions.iter().collect();
        assert_eq!(
            unique.len(),
            actions.len(),
            "duplicate action in {statement}"
        );
    }
}

/// Permanent version deletion belongs exclusively to the temporary elevated
/// credentials used for full teardown. `teardown_credentials.rs` asserts the
/// manifest never asks for it; this asserts the rendered policy never grants
/// it, which is the half that reaches AWS.
#[test]
fn the_policy_never_grants_permanent_version_deletion() {
    assert!(!granted_actions(&policy()).contains("s3:DeleteObjectVersion"));
}
