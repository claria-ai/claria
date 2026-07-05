//! Prefix listing against the in-process mock S3: a filename search prefix
//! appended to the client records prefix narrows ListObjectsV2 results.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_s3::primitives::ByteStream;
use claria_core::s3_keys;
use claria_mock_aws::testing::MockServer;
use uuid::Uuid;

const BUCKET: &str = "claria-test-bucket";

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

async fn seed(s3: &aws_sdk_s3::Client, id: Uuid, filenames: &[&str]) {
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
    for filename in filenames {
        s3.put_object()
            .bucket(BUCKET)
            .key(s3_keys::client_record_file(id, filename))
            .body(ByteStream::from_static(b"data"))
            .send()
            .await
            .expect("put object");
    }
}

#[tokio::test]
async fn search_prefix_narrows_listing() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    let id = Uuid::new_v4();
    seed(
        &s3,
        id,
        &[
            "Damien.pdf",
            "Damien.pdf.text",
            "Damon.txt",
            "Zelda.txt",
            "chat-history/abc.json",
        ],
    )
    .await;

    let prefix = s3_keys::client_records_search_prefix(id, "Dam");
    let objects = claria_storage::objects::list_objects_with_metadata(&s3, BUCKET, &prefix)
        .await
        .expect("list");

    let mut keys: Vec<&str> = objects.iter().map(|o| o.key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            s3_keys::client_record_file(id, "Damien.pdf"),
            s3_keys::client_record_file(id, "Damien.pdf.text"),
            s3_keys::client_record_file(id, "Damon.txt"),
        ]
    );
}

#[tokio::test]
async fn empty_search_prefix_lists_everything() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    let id = Uuid::new_v4();
    seed(&s3, id, &["Damien.pdf", "Zelda.txt"]).await;

    let prefix = s3_keys::client_records_prefix(id);
    let objects = claria_storage::objects::list_objects_with_metadata(&s3, BUCKET, &prefix)
        .await
        .expect("list");
    assert_eq!(objects.len(), 2);
}

#[tokio::test]
async fn prefix_filtered_listing_hides_matching_sidecars() {
    let server = MockServer::spawn().await;
    let sdk = build_sdk_config(&server.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);

    let id = Uuid::new_v4();
    seed(&s3, id, &["Damien.pdf", "Damien.pdf.text", "Damon.txt"]).await;

    // Prefix matches base + sidecar: the sidecar is hidden.
    let prefix = s3_keys::client_records_search_prefix(id, "Dam");
    let objects = claria_storage::objects::list_objects_with_metadata(&s3, BUCKET, &prefix)
        .await
        .expect("list");
    let all_keys: std::collections::HashSet<&str> =
        objects.iter().map(|o| o.key.as_str()).collect();
    let mut visible: Vec<&str> = objects
        .iter()
        .map(|o| o.key.as_str())
        .filter(|k| !s3_keys::is_hidden_sidecar(k, &all_keys))
        .collect();
    visible.sort_unstable();
    assert_eq!(
        visible,
        vec![
            s3_keys::client_record_file(id, "Damien.pdf"),
            s3_keys::client_record_file(id, "Damon.txt"),
        ]
    );

    // Prefix extends past the base filename into `.text`: only the sidecar
    // matches, so it is shown.
    let prefix = s3_keys::client_records_search_prefix(id, "Damien.pdf.te");
    let objects = claria_storage::objects::list_objects_with_metadata(&s3, BUCKET, &prefix)
        .await
        .expect("list");
    let all_keys: std::collections::HashSet<&str> =
        objects.iter().map(|o| o.key.as_str()).collect();
    let visible: Vec<&str> = objects
        .iter()
        .map(|o| o.key.as_str())
        .filter(|k| !s3_keys::is_hidden_sidecar(k, &all_keys))
        .collect();
    assert_eq!(
        visible,
        vec![s3_keys::client_record_file(id, "Damien.pdf.text")]
    );
}
