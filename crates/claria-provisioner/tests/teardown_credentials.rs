//! Least-privilege boundaries around permanent infrastructure teardown.

use std::collections::HashSet;

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::{scenarios, testing::MockServer};
use claria_provisioner::{
    CallerIdentity, CredentialAssessment, CredentialClass, ProvisionerError, build_manifest,
    update_iam_policy, validate_teardown_credentials,
};

const ACCOUNT_ID: &str = "185735714230";
const POLICY_ARN: &str = "arn:aws:iam::185735714230:policy/ClariaProvisionerAccess";

fn assessment(account_id: &str, credential_class: CredentialClass) -> CredentialAssessment {
    CredentialAssessment {
        identity: CallerIdentity {
            account_id: account_id.to_string(),
            arn: format!("arn:aws:iam::{account_id}:root"),
            user_id: account_id.to_string(),
            is_root: credential_class == CredentialClass::Root,
        },
        credential_class,
        reason: "test fixture".to_string(),
    }
}

fn build_sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new(
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        None,
        None,
        "claria-test",
    );
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

fn policy_actions(document: &str) -> HashSet<String> {
    let value: serde_json::Value = serde_json::from_str(document).expect("valid policy JSON");
    let mut actions = HashSet::new();

    for statement in value["Statement"].as_array().expect("policy statements") {
        match &statement["Action"] {
            serde_json::Value::String(action) => {
                actions.insert(action.clone());
            }
            serde_json::Value::Array(values) => {
                actions.extend(
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string)),
                );
            }
            _ => {}
        }
    }

    actions
}

#[test]
fn root_and_iam_admin_credentials_can_authorize_teardown() {
    for credential_class in [CredentialClass::Root, CredentialClass::IamAdmin] {
        validate_teardown_credentials(&assessment(ACCOUNT_ID, credential_class), ACCOUNT_ID)
            .expect("elevated credential in the configured account");
    }
}

#[test]
fn scoped_and_insufficient_credentials_cannot_authorize_teardown() {
    for credential_class in [CredentialClass::ScopedClaria, CredentialClass::Insufficient] {
        let error =
            validate_teardown_credentials(&assessment(ACCOUNT_ID, credential_class), ACCOUNT_ID)
                .expect_err("routine credentials must not authorize permanent teardown");
        assert!(
            matches!(error, ProvisionerError::TeardownCredentialsRequired),
            "got {error:?}"
        );
    }
}

#[test]
fn elevated_credentials_from_another_account_cannot_authorize_teardown() {
    let error = validate_teardown_credentials(
        &assessment("999999999999", CredentialClass::Root),
        ACCOUNT_ID,
    )
    .expect_err("cross-account credentials must fail closed");

    assert!(
        matches!(
            error,
            ProvisionerError::CredentialAccountMismatch {
                ref expected_account_id,
                ref actual_account_id,
            } if expected_account_id == ACCOUNT_ID && actual_account_id == "999999999999"
        ),
        "got {error:?}"
    );
}

#[test]
fn scoped_policy_manifest_does_not_request_permanent_version_deletion() {
    let manifest = build_manifest(ACCOUNT_ID, "claria", "us-east-1");
    let actions: HashSet<&str> = manifest
        .specs
        .iter()
        .flat_map(|spec| spec.iam_actions.iter().map(String::as_str))
        .collect();

    assert!(!actions.contains("s3:DeleteObjectVersion"));
    assert!(actions.contains("s3:DeleteObject"));
}

#[tokio::test]
async fn policy_update_revokes_permanent_version_deletion() {
    let server = MockServer::spawn().await;
    {
        let mut state = server.state.write().await;
        scenarios::load("bootstrapped", &mut state).expect("seed IAM policy");
        let policy = state.policies.get_mut(POLICY_ARN).expect("seeded policy");
        policy.versions[0].document = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Action": ["s3:DeleteObject", "s3:DeleteObjectVersion"],
                "Resource": "*",
            }],
        })
        .to_string();
    }

    update_iam_policy(&build_sdk_config(&server.endpoint), "claria", ACCOUNT_ID)
        .await
        .expect("update scoped policy");

    let state = server.state.read().await;
    let policy = state.policies.get(POLICY_ARN).expect("updated policy");
    let version = policy
        .versions
        .iter()
        .find(|version| version.version_id == policy.default_version_id)
        .expect("default policy version");
    let actions = policy_actions(&version.document);

    assert!(!actions.contains("s3:DeleteObjectVersion"));
    assert!(actions.contains("s3:DeleteObject"));
}
