//! Tests for the concurrent S3 listing helpers in `claria_desktop::records`,
//! driven end-to-end against `claria-mock-aws`.

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_sdk_s3::primitives::ByteStream;

use claria_desktop::record_cache::RecordCache;
use claria_desktop::records::{
    ClientSummary, fetch_record_texts, get_client_record_details, list_client_summaries,
    search_record_contents, update_client_name, validate_client_name,
};
use claria_mock_aws::testing::MockServer;

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

async fn setup() -> (MockServer, aws_sdk_s3::Client) {
    let mock = MockServer::spawn().await;
    let sdk = build_sdk_config(&mock.endpoint);
    let s3 = claria_storage::client::from_config(&sdk);
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
    (mock, s3)
}

async fn put(s3: &aws_sdk_s3::Client, key: &str, body: impl Into<Vec<u8>>) {
    s3.put_object()
        .bucket(BUCKET)
        .key(key)
        .body(ByteStream::from(body.into()))
        .send()
        .await
        .expect("put object");
}

fn client_json(id: uuid::Uuid, name: &str) -> Vec<u8> {
    let client = claria_core::models::client::Client {
        id,
        name: name.to_string(),
        created_at: jiff::Timestamp::from_second(1_700_000_000).unwrap(),
        updated_at: jiff::Timestamp::from_second(1_700_000_000).unwrap(),
    };
    serde_json::to_vec(&client).unwrap()
}

#[tokio::test]
async fn list_client_summaries_returns_all_clients_sorted_by_created_at_desc() {
    let (_mock, s3) = setup().await;

    // Seed 40 clients with distinct creation times, in scrambled key order.
    let count = 40;
    for i in 0..count {
        let client = claria_core::models::client::Client {
            id: uuid::Uuid::new_v4(),
            name: format!("Client {i:02}"),
            created_at: jiff::Timestamp::from_second(1_700_000_000 + i * 60).unwrap(),
            updated_at: jiff::Timestamp::from_second(1_700_000_000 + i * 60).unwrap(),
        };
        let key = claria_core::s3_keys::client(client.id);
        put(&s3, &key, serde_json::to_vec(&client).unwrap()).await;
    }

    let cache = RecordCache::new();
    let summaries = list_client_summaries(&s3, BUCKET, &cache)
        .await
        .expect("list_client_summaries");

    assert_eq!(summaries.len(), count as usize);

    // Most recently created first.
    let names: Vec<&str> = summaries.iter().map(|c| c.name.as_str()).collect();
    let expected: Vec<String> = (0..count).rev().map(|i| format!("Client {i:02}")).collect();
    assert_eq!(
        names,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );

    for window in summaries.windows(2) {
        assert!(window[0].created_at >= window[1].created_at);
    }
}

#[tokio::test]
async fn client_record_details_count_files_and_current_storage() {
    let (_mock, s3) = setup().await;
    let id = uuid::Uuid::new_v4();
    put(
        &s3,
        &claria_core::s3_keys::client(id),
        client_json(id, "Jane Doe"),
    )
    .await;

    let records_prefix = claria_core::s3_keys::client_records_prefix(id);
    put(&s3, &format!("{records_prefix}notes.txt"), vec![b'a'; 10]).await;
    put(&s3, &format!("{records_prefix}scan.pdf"), vec![b'b'; 20]).await;
    put(
        &s3,
        &format!("{records_prefix}scan.pdf.text"),
        vec![b'c'; 30],
    )
    .await;
    put(
        &s3,
        &format!("{records_prefix}chat-history/chat.json"),
        vec![b'd'; 40],
    )
    .await;
    put(
        &s3,
        &claria_core::s3_keys::report_workspace(id),
        vec![b'e'; 50],
    )
    .await;

    let details = get_client_record_details(&s3, BUCKET, id)
        .await
        .expect("client record details");

    assert_eq!(details.id, id.to_string());
    assert_eq!(details.name, "Jane Doe");
    assert_eq!(details.file_count, 2);
    assert_eq!(details.storage_bytes, 150);
    assert_eq!(details.created_at, "2023-11-14T22:13:20Z");
}

#[tokio::test]
async fn update_client_name_trims_and_preserves_client_identity() {
    let (_mock, s3) = setup().await;
    let id = uuid::Uuid::new_v4();
    let key = claria_core::s3_keys::client(id);
    put(&s3, &key, client_json(id, "Original Name")).await;

    let updated = update_client_name(&s3, BUCKET, id, "  Renamed Client  ")
        .await
        .expect("rename client");
    assert_eq!(updated.id, id.to_string());
    assert_eq!(updated.name, "Renamed Client");
    assert_eq!(updated.created_at, "2023-11-14T22:13:20Z");

    let stored = claria_storage::objects::get_object(&s3, BUCKET, &key)
        .await
        .expect("renamed client object");
    let client: claria_core::models::client::Client =
        serde_json::from_slice(&stored.body).expect("client json");
    assert_eq!(client.id, id);
    assert_eq!(client.name, "Renamed Client");
    assert_eq!(
        client.created_at,
        jiff::Timestamp::from_second(1_700_000_000).expect("timestamp")
    );
    assert!(client.updated_at > client.created_at);
}

#[test]
fn client_name_validation_rejects_blank_and_oversized_names() {
    assert_eq!(
        validate_client_name("   ").expect_err("blank name"),
        "Client name cannot be empty."
    );
    assert_eq!(
        validate_client_name(&"x".repeat(201)).expect_err("oversized name"),
        "Client name cannot exceed 200 characters."
    );
}

#[tokio::test]
async fn list_client_summaries_skips_unparseable_objects() {
    let (_mock, s3) = setup().await;

    let client = claria_core::models::client::Client {
        id: uuid::Uuid::new_v4(),
        name: "Valid Client".into(),
        created_at: jiff::Timestamp::from_second(1_700_000_000).unwrap(),
        updated_at: jiff::Timestamp::from_second(1_700_000_000).unwrap(),
    };
    put(
        &s3,
        &claria_core::s3_keys::client(client.id),
        serde_json::to_vec(&client).unwrap(),
    )
    .await;
    put(&s3, "clients/garbage.json", b"not json {".to_vec()).await;

    let cache = RecordCache::new();
    let summaries = list_client_summaries(&s3, BUCKET, &cache)
        .await
        .expect("list_client_summaries");

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "Valid Client");
    assert_eq!(summaries[0].id, client.id.to_string());
}

#[tokio::test]
async fn fetch_record_texts_reads_txt_and_sidecars_in_listing_order() {
    let (_mock, s3) = setup().await;
    let id = uuid::Uuid::new_v4();
    let prefix = claria_core::s3_keys::client_records_prefix(id);

    put(&s3, &format!("{prefix}a.txt"), b"alpha text".to_vec()).await;
    put(&s3, &format!("{prefix}b.pdf"), b"%PDF binary".to_vec()).await;
    put(
        &s3,
        &format!("{prefix}b.pdf.text"),
        b"bravo extracted".to_vec(),
    )
    .await;
    // No sidecar: listed with no text.
    put(&s3, &format!("{prefix}c.pdf"), b"%PDF binary".to_vec()).await;
    // Chat history is never record context.
    put(
        &s3,
        &format!("{prefix}chat-history/chat1.json"),
        b"{}".to_vec(),
    )
    .await;
    // A different client's records must not leak in.
    let other_prefix = claria_core::s3_keys::client_records_prefix(uuid::Uuid::new_v4());
    put(&s3, &format!("{other_prefix}z.txt"), b"other".to_vec()).await;

    let cache = RecordCache::new();
    let texts = fetch_record_texts(&s3, BUCKET, id, &cache)
        .await
        .expect("fetch_record_texts");

    assert_eq!(
        texts,
        vec![
            ("a.txt".to_string(), Some("alpha text".to_string())),
            ("b.pdf".to_string(), Some("bravo extracted".to_string())),
            ("c.pdf".to_string(), None),
        ]
    );
}

#[tokio::test]
async fn search_record_contents_matches_text_case_insensitively() {
    let (_mock, s3) = setup().await;
    let id = uuid::Uuid::new_v4();
    let prefix = claria_core::s3_keys::client_records_prefix(id);

    put(
        &s3,
        &format!("{prefix}notes.txt"),
        b"Patient reports MIGRAINE symptoms".to_vec(),
    )
    .await;
    put(&s3, &format!("{prefix}intake.pdf"), b"%PDF binary".to_vec()).await;
    put(
        &s3,
        &format!("{prefix}intake.pdf.text"),
        b"History of migraine and tension headache".to_vec(),
    )
    .await;
    put(&s3, &format!("{prefix}scan.pdf"), b"%PDF binary".to_vec()).await;
    put(
        &s3,
        &format!("{prefix}scan.pdf.text"),
        b"no relevant terms here".to_vec(),
    )
    .await;

    let cache = RecordCache::new();
    let hits = search_record_contents(&s3, BUCKET, id, &cache, "Migraine")
        .await
        .expect("search_record_contents");

    assert_eq!(hits, vec!["intake.pdf".to_string(), "notes.txt".to_string()]);
}

#[tokio::test]
async fn search_record_contents_ignores_filenames_and_blank_queries() {
    let (_mock, s3) = setup().await;
    let id = uuid::Uuid::new_v4();
    let prefix = claria_core::s3_keys::client_records_prefix(id);

    put(
        &s3,
        &format!("{prefix}notes.txt"),
        b"nothing of interest".to_vec(),
    )
    .await;

    let cache = RecordCache::new();

    // A query matching only the filename is not a content match.
    let hits = search_record_contents(&s3, BUCKET, id, &cache, "notes")
        .await
        .expect("filename query");
    assert!(hits.is_empty());

    // Blank queries match nothing.
    let hits = search_record_contents(&s3, BUCKET, id, &cache, "   ")
        .await
        .expect("blank query");
    assert!(hits.is_empty());
}

#[tokio::test]
async fn fetch_record_texts_returns_empty_for_client_with_no_records() {
    let (_mock, s3) = setup().await;

    let cache = RecordCache::new();
    let texts = fetch_record_texts(&s3, BUCKET, uuid::Uuid::new_v4(), &cache)
        .await
        .expect("fetch_record_texts");

    assert!(texts.is_empty());
}

#[tokio::test]
async fn cache_invalidates_when_object_etag_changes() {
    let (_mock, s3) = setup().await;
    let cache = RecordCache::new();

    let id = uuid::Uuid::new_v4();
    let key = claria_core::s3_keys::client(id);
    put(&s3, &key, client_json(id, "Original")).await;

    // Warm the cache.
    let first = list_client_summaries(&s3, BUCKET, &cache)
        .await
        .expect("first list");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].name, "Original");

    // Overwrite with new content → new body → new ETag.
    put(&s3, &key, client_json(id, "Renamed")).await;

    // The listing's fresh ETag must invalidate the cached entry.
    let second = list_client_summaries(&s3, BUCKET, &cache)
        .await
        .expect("second list");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].name, "Renamed");
}

#[tokio::test]
async fn cache_hit_returns_identical_summaries_on_unchanged_data() {
    let (_mock, s3) = setup().await;
    let cache = RecordCache::new();

    for i in 0..5 {
        let id = uuid::Uuid::new_v4();
        put(
            &s3,
            &claria_core::s3_keys::client(id),
            client_json(id, &format!("Client {i}")),
        )
        .await;
    }

    let first = list_client_summaries(&s3, BUCKET, &cache)
        .await
        .expect("first list");
    let second = list_client_summaries(&s3, BUCKET, &cache)
        .await
        .expect("second list");

    assert_eq!(first.len(), 5);
    let project = |v: &[ClientSummary]| {
        v.iter()
            .map(|c| (c.id.clone(), c.name.clone(), c.created_at.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(project(&first), project(&second));
}

#[tokio::test]
async fn get_or_fetch_serves_cached_bytes_when_etag_matches() {
    let (_mock, s3) = setup().await;
    let cache = RecordCache::new();

    let id = uuid::Uuid::new_v4();
    let key = claria_core::s3_keys::client(id);
    let body = client_json(id, "Cached");
    put(&s3, &key, body.clone()).await;

    // Grab the ETag the listing reports, then warm the cache with it.
    let etags = claria_storage::objects::list_object_etags(
        &s3,
        BUCKET,
        claria_core::s3_keys::CLIENTS_PREFIX,
    )
    .await
    .expect("list etags");
    let (_, etag) = etags
        .iter()
        .find(|(k, _)| *k == key)
        .expect("listed key")
        .clone();

    let warmed = cache
        .get_or_fetch(&s3, BUCKET, &key, &etag)
        .await
        .expect("warm");
    assert_eq!(&*warmed, body.as_slice());

    // Delete the object from S3 so any real GET would 404. A matching ETag must
    // still serve the cached bytes.
    s3.delete_object()
        .bucket(BUCKET)
        .key(&key)
        .send()
        .await
        .expect("delete");

    let hit = cache
        .get_or_fetch(&s3, BUCKET, &key, &etag)
        .await
        .expect("cache hit after delete");
    assert_eq!(&*hit, body.as_slice());

    // A mismatched (or empty) ETag must NOT be served from cache: it falls
    // through to a GET, which now 404s.
    assert!(cache.get_or_fetch(&s3, BUCKET, &key, "").await.is_err());
    assert!(
        cache
            .get_or_fetch(&s3, BUCKET, &key, "\"different\"")
            .await
            .is_err()
    );
}
