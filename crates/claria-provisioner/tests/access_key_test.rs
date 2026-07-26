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
