mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request, request_with_header};

// ── Bucket lifecycle ──────────────────────────────────────────────────

#[tokio::test]
async fn create_and_head_bucket() {
    let app = app();
    let r = request(&app, Method::PUT, "/test-bucket", "").await;
    assert_eq!(r.status, StatusCode::OK);

    let r = request(&app, Method::HEAD, "/test-bucket", "").await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn head_nonexistent_bucket_returns_404() {
    let app = app();
    let r = request(&app, Method::HEAD, "/no-such-bucket", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_bucket_with_location_constraint() {
    let app = app();
    let body = r#"<CreateBucketConfiguration><LocationConstraint>eu-west-1</LocationConstraint></CreateBucketConfiguration>"#;
    let r = request(&app, Method::PUT, "/regional-bucket", body).await;
    assert_eq!(r.status, StatusCode::OK);

    // Bucket should be accessible
    let r = request(&app, Method::HEAD, "/regional-bucket", "").await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn delete_bucket_removes_an_empty_bucket() {
    let app = app();
    request(&app, Method::PUT, "/del-bucket", "").await;
    request_with_header(&app, Method::PUT, "/del-bucket/key1", "content-type", "text/plain", "data").await;
    request(&app, Method::DELETE, "/del-bucket/key1", "").await;

    let r = request(&app, Method::DELETE, "/del-bucket", "").await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);

    let r = request(&app, Method::HEAD, "/del-bucket", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_bucket_rejects_a_bucket_holding_objects() {
    let app = app();
    request(&app, Method::PUT, "/full-bucket", "").await;
    request_with_header(&app, Method::PUT, "/full-bucket/key1", "content-type", "text/plain", "data").await;

    let r = request(&app, Method::DELETE, "/full-bucket", "").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.body.contains("BucketNotEmpty"));

    let r = request(&app, Method::HEAD, "/full-bucket", "").await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn delete_bucket_rejects_a_bucket_holding_only_delete_markers() {
    let app = app();
    request(&app, Method::PUT, "/marker-bucket", "").await;
    request(&app, Method::PUT, "/marker-bucket?versioning",
        r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#).await;
    request_with_header(&app, Method::PUT, "/marker-bucket/key1", "content-type", "text/plain", "data").await;
    request(&app, Method::DELETE, "/marker-bucket/key1", "").await;

    let r = request(&app, Method::DELETE, "/marker-bucket", "").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.body.contains("BucketNotEmpty"));
}

// ── Object CRUD ──────────────────────────────────────────────────────

#[tokio::test]
async fn put_and_get_object() {
    let app = app();
    request(&app, Method::PUT, "/mybucket", "").await;

    let r = request_with_header(
        &app, Method::PUT, "/mybucket/hello.txt",
        "content-type", "text/plain", "hello world",
    ).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.header("etag").is_some());

    let r = request(&app, Method::GET, "/mybucket/hello.txt", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, "hello world");
    assert_eq!(r.header("content-type").unwrap(), "text/plain");
}

#[tokio::test]
async fn head_object_returns_metadata_without_body() {
    let app = app();
    request(&app, Method::PUT, "/hbucket", "").await;
    request_with_header(&app, Method::PUT, "/hbucket/file.bin", "content-type", "application/octet-stream", "binary data").await;

    let r = request(&app, Method::HEAD, "/hbucket/file.bin", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.header("content-length").unwrap(), "11");
    assert!(r.body.is_empty());
}

#[tokio::test]
async fn get_nonexistent_object_returns_404() {
    let app = app();
    request(&app, Method::PUT, "/bucket404", "").await;

    let r = request(&app, Method::GET, "/bucket404/missing.txt", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("NoSuchKey"));
}

#[tokio::test]
async fn put_object_to_nonexistent_bucket_returns_404() {
    let app = app();
    let r = request_with_header(
        &app, Method::PUT, "/no-bucket/key",
        "content-type", "text/plain", "data",
    ).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("NoSuchBucket"));
}

#[tokio::test]
async fn delete_object_without_versioning_removes_it() {
    let app = app();
    request(&app, Method::PUT, "/delbucket", "").await;
    request_with_header(&app, Method::PUT, "/delbucket/obj", "content-type", "text/plain", "data").await;

    let r = request(&app, Method::DELETE, "/delbucket/obj", "").await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);

    let r = request(&app, Method::GET, "/delbucket/obj", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_overwrites_previous_version_when_unversioned() {
    let app = app();
    request(&app, Method::PUT, "/owbucket", "").await;
    request_with_header(&app, Method::PUT, "/owbucket/key", "content-type", "text/plain", "v1").await;
    request_with_header(&app, Method::PUT, "/owbucket/key", "content-type", "text/plain", "v2").await;

    let r = request(&app, Method::GET, "/owbucket/key", "").await;
    assert_eq!(r.body, "v2");
}

// ── Versioning ──────────────────────────────────────────────────────

#[tokio::test]
async fn versioning_defaults_to_unset() {
    let app = app();
    request(&app, Method::PUT, "/verbucket", "").await;

    let r = request(&app, Method::GET, "/verbucket?versioning", "").await;
    assert_eq!(r.status, StatusCode::OK);
    // Unset: no <Status> element
    assert!(!r.body.contains("<Status>"));
}

#[tokio::test]
async fn enable_and_get_versioning() {
    let app = app();
    request(&app, Method::PUT, "/verbucket2", "").await;

    let body = r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#;
    let r = request(&app, Method::PUT, "/verbucket2?versioning", body).await;
    assert_eq!(r.status, StatusCode::OK);

    let r = request(&app, Method::GET, "/verbucket2?versioning", "").await;
    assert!(r.body.contains("<Status>Enabled</Status>"));
}

#[tokio::test]
async fn versioning_on_nonexistent_bucket_returns_404() {
    let app = app();
    let r = request(&app, Method::GET, "/nonexist?versioning", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn versioned_put_creates_multiple_versions() {
    let app = app();
    request(&app, Method::PUT, "/vbucket", "").await;
    request(&app, Method::PUT, "/vbucket?versioning",
        r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#).await;

    let r1 = request_with_header(&app, Method::PUT, "/vbucket/doc.txt", "content-type", "text/plain", "version1").await;
    let vid1 = r1.header("x-amz-version-id").unwrap().to_string();

    let r2 = request_with_header(&app, Method::PUT, "/vbucket/doc.txt", "content-type", "text/plain", "version2").await;
    let vid2 = r2.header("x-amz-version-id").unwrap().to_string();

    assert_ne!(vid1, vid2);

    // Latest version
    let r = request(&app, Method::GET, "/vbucket/doc.txt", "").await;
    assert_eq!(r.body, "version2");

    // Specific old version
    let r = request(&app, Method::GET, &format!("/vbucket/doc.txt?versionId={vid1}"), "").await;
    assert_eq!(r.body, "version1");
}

#[tokio::test]
async fn versioned_delete_creates_delete_marker() {
    let app = app();
    request(&app, Method::PUT, "/vdel", "").await;
    request(&app, Method::PUT, "/vdel?versioning",
        r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#).await;

    let r = request_with_header(&app, Method::PUT, "/vdel/file.txt", "content-type", "text/plain", "data").await;
    let vid = r.header("x-amz-version-id").unwrap().to_string();

    // Delete (creates marker)
    request(&app, Method::DELETE, "/vdel/file.txt", "").await;

    // Old version still accessible by version ID
    let r = request(&app, Method::GET, &format!("/vdel/file.txt?versionId={vid}"), "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, "data");

    // Delete marker shows up in version listing
    let r = request(&app, Method::GET, "/vdel?versions", "").await;
    assert!(r.body.contains("<DeleteMarker>"));
}

// ── Encryption ──────────────────────────────────────────────────────

#[tokio::test]
async fn encryption_not_found_by_default() {
    let app = app();
    request(&app, Method::PUT, "/encbucket", "").await;

    let r = request(&app, Method::GET, "/encbucket?encryption", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("ServerSideEncryptionConfigurationNotFoundError"));
}

#[tokio::test]
async fn set_and_get_encryption() {
    let app = app();
    request(&app, Method::PUT, "/encbucket2", "").await;

    let body = r#"<ServerSideEncryptionConfiguration><Rule><ApplyServerSideEncryptionByDefault><SSEAlgorithm>AES256</SSEAlgorithm></ApplyServerSideEncryptionByDefault></Rule></ServerSideEncryptionConfiguration>"#;
    request(&app, Method::PUT, "/encbucket2?encryption", body).await;

    let r = request(&app, Method::GET, "/encbucket2?encryption", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<SSEAlgorithm>AES256</SSEAlgorithm>"));
}

// ── Public Access Block ─────────────────────────────────────────────

#[tokio::test]
async fn public_access_block_not_found_by_default() {
    let app = app();
    request(&app, Method::PUT, "/pabbucket", "").await;

    let r = request(&app, Method::GET, "/pabbucket?publicAccessBlock", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("NoSuchPublicAccessBlockConfiguration"));
}

#[tokio::test]
async fn set_get_delete_public_access_block() {
    let app = app();
    request(&app, Method::PUT, "/pab2", "").await;

    let body = r#"<PublicAccessBlockConfiguration>
        <BlockPublicAcls>true</BlockPublicAcls>
        <IgnorePublicAcls>true</IgnorePublicAcls>
        <BlockPublicPolicy>true</BlockPublicPolicy>
        <RestrictPublicBuckets>true</RestrictPublicBuckets>
    </PublicAccessBlockConfiguration>"#;
    request(&app, Method::PUT, "/pab2?publicAccessBlock", body).await;

    let r = request(&app, Method::GET, "/pab2?publicAccessBlock", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<BlockPublicAcls>true</BlockPublicAcls>"));

    let r = request(&app, Method::DELETE, "/pab2?publicAccessBlock", "").await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);

    let r = request(&app, Method::GET, "/pab2?publicAccessBlock", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ── Bucket Policy ───────────────────────────────────────────────────

#[tokio::test]
async fn bucket_policy_not_found_by_default() {
    let app = app();
    request(&app, Method::PUT, "/polbucket", "").await;

    let r = request(&app, Method::GET, "/polbucket?policy", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("NoSuchBucketPolicy"));
}

#[tokio::test]
async fn set_get_delete_bucket_policy() {
    let app = app();
    request(&app, Method::PUT, "/pol2", "").await;

    let policy = r#"{"Version":"2012-10-17","Statement":[]}"#;
    request(&app, Method::PUT, "/pol2?policy", policy).await;

    let r = request(&app, Method::GET, "/pol2?policy", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("2012-10-17"));

    request(&app, Method::DELETE, "/pol2?policy", "").await;

    let r = request(&app, Method::GET, "/pol2?policy", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ── ListObjectsV2 ───────────────────────────────────────────────────

#[tokio::test]
async fn list_objects_v2_empty_bucket() {
    let app = app();
    request(&app, Method::PUT, "/listbucket", "").await;

    let r = request(&app, Method::GET, "/listbucket?list-type=2", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<KeyCount>0</KeyCount>"));
    assert!(r.body.contains("<IsTruncated>false</IsTruncated>"));
}

#[tokio::test]
async fn list_objects_v2_with_prefix() {
    let app = app();
    request(&app, Method::PUT, "/listbucket2", "").await;
    request_with_header(&app, Method::PUT, "/listbucket2/a/one.txt", "content-type", "text/plain", "1").await;
    request_with_header(&app, Method::PUT, "/listbucket2/a/two.txt", "content-type", "text/plain", "2").await;
    request_with_header(&app, Method::PUT, "/listbucket2/b/three.txt", "content-type", "text/plain", "3").await;

    let r = request(&app, Method::GET, "/listbucket2?list-type=2&prefix=a/", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<KeyCount>2</KeyCount>"));
    assert!(r.body.contains("<Key>a/one.txt</Key>"));
    assert!(r.body.contains("<Key>a/two.txt</Key>"));
    assert!(!r.body.contains("<Key>b/three.txt</Key>"));
}

#[tokio::test]
async fn list_objects_v2_nonexistent_bucket() {
    let app = app();
    let r = request(&app, Method::GET, "/nope?list-type=2", "").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ── ListObjectVersions ──────────────────────────────────────────────

#[tokio::test]
async fn list_object_versions_shows_versions_and_delete_markers() {
    let app = app();
    request(&app, Method::PUT, "/lovbucket", "").await;
    request(&app, Method::PUT, "/lovbucket?versioning",
        r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#).await;

    request_with_header(&app, Method::PUT, "/lovbucket/file.txt", "content-type", "text/plain", "v1").await;
    request_with_header(&app, Method::PUT, "/lovbucket/file.txt", "content-type", "text/plain", "v2").await;
    request(&app, Method::DELETE, "/lovbucket/file.txt", "").await;

    let r = request(&app, Method::GET, "/lovbucket?versions", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<Version>"));
    assert!(r.body.contains("<DeleteMarker>"));
}

#[tokio::test]
async fn list_object_versions_pages_through_markers() {
    let app = app();
    request(&app, Method::PUT, "/paged", "").await;
    request(&app, Method::PUT, "/paged?versioning",
        r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#).await;

    for i in 0..5 {
        let uri = format!("/paged/key{i}.txt");
        request_with_header(&app, Method::PUT, &uri, "content-type", "text/plain", "data").await;
    }

    let r = request(&app, Method::GET, "/paged?versions&max-keys=2", "").await;
    assert!(r.body.contains("<IsTruncated>true</IsTruncated>"));
    assert!(r.body.contains("<Key>key0.txt</Key>"));
    assert!(r.body.contains("<Key>key1.txt</Key>"));
    assert!(!r.body.contains("<Key>key2.txt</Key>"));
    assert!(r.body.contains("<NextKeyMarker>key1.txt</NextKeyMarker>"));

    let r = request(
        &app,
        Method::GET,
        "/paged?versions&max-keys=2&key-marker=key1.txt",
        "",
    )
    .await;
    assert!(r.body.contains("<Key>key2.txt</Key>"));
    assert!(!r.body.contains("<Key>key1.txt</Key>"));
}

// ── DeleteObjects ───────────────────────────────────────────────────

#[tokio::test]
async fn delete_objects_removes_a_batch_of_versions() {
    let app = app();
    request(&app, Method::PUT, "/batch", "").await;
    request(&app, Method::PUT, "/batch?versioning",
        r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#).await;

    let r1 = request_with_header(&app, Method::PUT, "/batch/doc.txt", "content-type", "text/plain", "v1").await;
    let vid1 = r1.header("x-amz-version-id").unwrap().to_string();
    let r2 = request_with_header(&app, Method::PUT, "/batch/doc.txt", "content-type", "text/plain", "v2").await;
    let vid2 = r2.header("x-amz-version-id").unwrap().to_string();

    let body = format!(
        "<Delete><Object><Key>doc.txt</Key><VersionId>{vid1}</VersionId></Object>\
         <Object><Key>doc.txt</Key><VersionId>{vid2}</VersionId></Object></Delete>"
    );
    let r = request(&app, Method::POST, "/batch?delete", body).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<Deleted>"));
    assert!(!r.body.contains("<Error>"));

    // Both versions are gone, so the bucket is genuinely empty.
    let r = request(&app, Method::DELETE, "/batch", "").await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn delete_objects_without_version_ids_leaves_delete_markers() {
    let app = app();
    request(&app, Method::PUT, "/soft", "").await;
    request(&app, Method::PUT, "/soft?versioning",
        r#"<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>"#).await;
    request_with_header(&app, Method::PUT, "/soft/doc.txt", "content-type", "text/plain", "v1").await;

    let r = request(
        &app,
        Method::POST,
        "/soft?delete",
        "<Delete><Object><Key>doc.txt</Key></Object></Delete>",
    )
    .await;
    assert!(r.body.contains("<DeleteMarker>true</DeleteMarker>"));

    let r = request(&app, Method::DELETE, "/soft", "").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn delete_query_flag_is_not_confused_with_a_prefix() {
    let app = app();
    request(&app, Method::PUT, "/prefixed", "").await;
    request_with_header(&app, Method::PUT, "/prefixed/deleted-later.txt", "content-type", "text/plain", "data").await;

    let r = request(&app, Method::GET, "/prefixed?list-type=2&prefix=deleted", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<Key>deleted-later.txt</Key>"));
}
