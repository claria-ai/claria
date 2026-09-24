//! Access-key handling against the in-process mock IAM.
//!
//! The IAM two-key ceiling is the one `CreateAccessKey` failure the desktop
//! app can recover from, so it has to arrive as its own error variant rather
//! than folded into the generic AWS bucket.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_mock_aws::testing::MockServer;
use claria_provisioner::{
    MAX_ACCESS_KEYS_PER_USER, ProvisionerError, create_access_key, list_user_access_keys,
};

fn build_sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let creds = Credentials::new(
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        None,
        None,
        "claria-test",
    );
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(creds))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

#[tokio::test]
async fn third_access_key_is_classified_as_a_limit_not_a_generic_aws_error() {
    let server = MockServer::spawn().await;
    let config = build_sdk_config(&server.endpoint);

    for _ in 0..MAX_ACCESS_KEYS_PER_USER {
        create_access_key(&config)
            .await
            .expect("first two keys fit under the IAM ceiling");
    }

    let err = create_access_key(&config)
        .await
        .expect_err("a third key exceeds the IAM ceiling");

    match err {
        ProvisionerError::AccessKeyLimitExceeded { user_name, limit } => {
            assert_eq!(user_name, "claria-admin");
            assert_eq!(limit, MAX_ACCESS_KEYS_PER_USER);
        }
        other => panic!("expected AccessKeyLimitExceeded, got {other:?}"),
    }
}

#[tokio::test]
async fn limit_message_names_the_user_and_the_way_out() {
    let message = ProvisionerError::AccessKeyLimitExceeded {
        user_name: "claria-admin".to_string(),
        limit: MAX_ACCESS_KEYS_PER_USER,
    }
    .to_string();

    assert!(message.contains("claria-admin"), "{message}");
    assert!(message.contains("delete one"), "{message}");
}

#[tokio::test]
async fn listing_returns_every_key_the_user_holds() {
    let server = MockServer::spawn().await;
    let config = build_sdk_config(&server.endpoint);

    let (first, _) = create_access_key(&config).await.expect("create first key");
    let (second, _) = create_access_key(&config).await.expect("create second key");

    let keys = list_user_access_keys(&config).await.expect("list keys");

    let ids: Vec<&str> = keys.iter().map(|k| k.access_key_id.as_str()).collect();
    assert!(ids.contains(&first.as_str()), "{ids:?}");
    assert!(ids.contains(&second.as_str()), "{ids:?}");
    assert_eq!(keys.len(), 2);
}

/// Deleting the root account's own key must omit `UserName`: that is what
/// makes `DeleteAccessKey` act on the caller rather than on a named user.
///
/// Setup promises the operator that Claria removes their root access key once
/// the scoped user is in place. The step existed in `bootstrap_account` and
/// nothing called it, so this pins the entry point the shipping path uses.
#[tokio::test]
async fn deleting_the_root_key_names_no_user() {
    let server = MockServer::spawn().await;
    let config = build_sdk_config(&server.endpoint);

    server.state.write().await.access_keys.insert(
        "AKIAROOTKEY000000001".to_string(),
        claria_mock_aws::state::AccessKeyRecord {
            access_key_id: "AKIAROOTKEY000000001".to_string(),
            secret_access_key: "not-a-real-secret".to_string(),
            // The root user's key belongs to no IAM user.
            user_name: String::new(),
            status: "Active".to_string(),
            create_date: "2026-01-01T00:00:00Z".to_string(),
            last_used_date: None,
            last_used_service: None,
        },
    );

    claria_provisioner::delete_root_access_key(&config, "AKIAROOTKEY000000001")
        .await
        .expect("the root key is deleted");

    assert!(
        !server
            .state
            .read()
            .await
            .access_keys
            .contains_key("AKIAROOTKEY000000001"),
        "the root access key is gone from the account"
    );
}

/// A failed delete is an error the caller can report, not a silent success.
/// Setup treats it as non-fatal and tells the operator to remove the key by
/// hand — which it can only do if the failure reaches it.
#[tokio::test]
async fn a_refused_root_key_delete_surfaces() {
    let server = MockServer::spawn().await;
    server
        .state
        .write()
        .await
        .operation_failures
        .insert("DeleteAccessKey".to_string(), "AccessDenied".to_string());
    let config = build_sdk_config(&server.endpoint);

    let err = claria_provisioner::delete_root_access_key(&config, "AKIAROOTKEY000000001")
        .await
        .expect_err("a refused delete must not read as a deletion");

    assert!(
        err.to_string().contains("iam:DeleteAccessKey"),
        "the operator is told which call was refused: {err}"
    );
}

/// The counterpart: `delete_user_access_key` names `claria-admin`, and that
/// is load-bearing. A key the Claria user does not own is not its to delete.
#[tokio::test]
async fn deleting_a_user_key_will_not_reach_a_key_that_user_does_not_own() {
    let server = MockServer::spawn().await;
    let config = build_sdk_config(&server.endpoint);

    server.state.write().await.access_keys.insert(
        "AKIASOMEONEELSE00001".to_string(),
        claria_mock_aws::state::AccessKeyRecord {
            access_key_id: "AKIASOMEONEELSE00001".to_string(),
            secret_access_key: "not-a-real-secret".to_string(),
            user_name: "someone-else".to_string(),
            status: "Active".to_string(),
            create_date: "2026-01-01T00:00:00Z".to_string(),
            last_used_date: None,
            last_used_service: None,
        },
    );

    claria_provisioner::delete_user_access_key(&config, "AKIASOMEONEELSE00001")
        .await
        .expect_err("another user's key is not Claria's to delete");

    assert!(
        server
            .state
            .read()
            .await
            .access_keys
            .contains_key("AKIASOMEONEELSE00001"),
        "the other user's key is untouched"
    );
}
