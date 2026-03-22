use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use uuid::Uuid;

use crate::{
    state::{BucketState, ObjectVersion, PublicAccessBlock, SharedState, VersioningStatus},
    xml,
};

/// Dispatch an S3 request by method + path + query params.
pub async fn dispatch(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    state: SharedState,
    body: Bytes,
) -> Response {
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();

    // Parse bucket and key from path-style URL: /{bucket} or /{bucket}/{key...}
    let trimmed = path.trim_start_matches('/');
    let (bucket, key) = match trimmed.split_once('/') {
        Some((b, k)) => (b.to_string(), Some(k.to_string())),
        None => (trimmed.to_string(), None),
    };

    if bucket.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing bucket name").into_response();
    }

    // Bucket-level operations (no key)
    if key.is_none() || key.as_deref() == Some("") {
        return dispatch_bucket(&method, &bucket, &query, state, body).await;
    }

    // Object-level operations
    let key = key.unwrap();
    dispatch_object(&method, &bucket, &key, &query, &headers, state, body).await
}

async fn dispatch_bucket(
    method: &Method,
    bucket: &str,
    query: &str,
    state: SharedState,
    body: Bytes,
) -> Response {
    // Route by query param presence
    if query.contains("versioning") {
        return match *method {
            Method::GET => get_bucket_versioning(bucket, state).await,
            Method::PUT => put_bucket_versioning(bucket, state, body).await,
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        };
    }
    if query.contains("encryption") {
        return match *method {
            Method::GET => get_bucket_encryption(bucket, state).await,
            Method::PUT => put_bucket_encryption(bucket, state, body).await,
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        };
    }
    if query.contains("publicAccessBlock") {
        return match *method {
            Method::GET => get_public_access_block(bucket, state).await,
            Method::PUT => put_public_access_block(bucket, state, body).await,
            Method::DELETE => delete_public_access_block(bucket, state).await,
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        };
    }
    if query.contains("policy") {
        return match *method {
            Method::GET => get_bucket_policy(bucket, state).await,
            Method::PUT => put_bucket_policy(bucket, state, body).await,
            Method::DELETE => delete_bucket_policy(bucket, state).await,
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        };
    }
    if query.contains("list-type=2") {
        return list_objects_v2(bucket, query, state).await;
    }
    if query.contains("versions") {
        return list_object_versions(bucket, query, state).await;
    }

    match *method {
        Method::HEAD => head_bucket(bucket, state).await,
        Method::PUT => create_bucket(bucket, state, body).await,
        Method::DELETE => delete_bucket(bucket, state).await,
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

async fn dispatch_object(
    method: &Method,
    bucket: &str,
    key: &str,
    query: &str,
    headers: &HeaderMap,
    state: SharedState,
    body: Bytes,
) -> Response {
    // Decode percent-encoded key
    let key = percent_encoding::percent_decode_str(key)
        .decode_utf8_lossy()
        .to_string();

    match *method {
        Method::HEAD => head_object(bucket, &key, query, state).await,
        Method::GET => get_object(bucket, &key, query, state).await,
        Method::PUT => put_object(bucket, &key, headers, state, body).await,
        Method::DELETE => delete_object(bucket, &key, state).await,
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

// ── Bucket operations ──

async fn head_bucket(bucket: &str, state: SharedState) -> Response {
    let st = state.read().await;
    if st.buckets.contains_key(bucket) {
        StatusCode::OK.into_response()
    } else {
        (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "The specified bucket does not exist")).into_response()
    }
}

async fn create_bucket(bucket: &str, state: SharedState, body: Bytes) -> Response {
    let region = if body.is_empty() {
        "us-east-1".to_string()
    } else {
        // Parse <CreateBucketConfiguration><LocationConstraint>REGION</...>
        let text = String::from_utf8_lossy(&body);
        extract_xml_value(&text, "LocationConstraint")
            .unwrap_or_else(|| "us-east-1".to_string())
    };

    let mut st = state.write().await;
    st.buckets.entry(bucket.to_string()).or_insert(BucketState {
        region,
        ..Default::default()
    });
    StatusCode::OK.into_response()
}

async fn delete_bucket(bucket: &str, state: SharedState) -> Response {
    let mut st = state.write().await;
    st.buckets.remove(bucket);
    // Remove all objects in the bucket
    st.objects.retain(|(b, _), _| b != bucket);
    StatusCode::NO_CONTENT.into_response()
}

// ── Versioning ──

async fn get_bucket_versioning(bucket: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let status_str = match st.buckets.get(bucket) {
        Some(b) => match b.versioning {
            VersioningStatus::Enabled => "Enabled",
            VersioningStatus::Suspended => "Suspended",
            VersioningStatus::Unset => "",
        },
        None => return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response(),
    };

    let body = if status_str.is_empty() {
        xml::xml_doc(&xml::wrap_ns(
            "VersioningConfiguration",
            "http://s3.amazonaws.com/doc/2006-03-01/",
            "",
        ))
    } else {
        xml::xml_doc(&xml::wrap_ns(
            "VersioningConfiguration",
            "http://s3.amazonaws.com/doc/2006-03-01/",
            &xml::el("Status", status_str),
        ))
    };
    (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
}

async fn put_bucket_versioning(bucket: &str, state: SharedState, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let status = if text.contains("Enabled") {
        VersioningStatus::Enabled
    } else {
        VersioningStatus::Suspended
    };

    let mut st = state.write().await;
    if let Some(b) = st.buckets.get_mut(bucket) {
        b.versioning = status;
        StatusCode::OK.into_response()
    } else {
        (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response()
    }
}

// ── Encryption ──

async fn get_bucket_encryption(bucket: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let b = match st.buckets.get(bucket) {
        Some(b) => b,
        None => return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response(),
    };

    match &b.encryption_algorithm {
        Some(algo) => {
            let body = xml::xml_doc(&xml::wrap_ns(
                "ServerSideEncryptionConfiguration",
                "http://s3.amazonaws.com/doc/2006-03-01/",
                &xml::wrap(
                    "Rule",
                    &xml::wrap(
                        "ApplyServerSideEncryptionByDefault",
                        &xml::el("SSEAlgorithm", algo),
                    ),
                ),
            ));
            (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
        }
        None => {
            (
                StatusCode::NOT_FOUND,
                xml::error_xml(
                    "ServerSideEncryptionConfigurationNotFoundError",
                    "The server side encryption configuration was not found",
                ),
            )
                .into_response()
        }
    }
}

async fn put_bucket_encryption(bucket: &str, state: SharedState, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let algo = extract_xml_value(&text, "SSEAlgorithm").unwrap_or("AES256".to_string());

    let mut st = state.write().await;
    if let Some(b) = st.buckets.get_mut(bucket) {
        b.encryption_algorithm = Some(algo);
        StatusCode::OK.into_response()
    } else {
        (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response()
    }
}

// ── Public Access Block ──

async fn get_public_access_block(bucket: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let b = match st.buckets.get(bucket) {
        Some(b) => b,
        None => return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response(),
    };

    match &b.public_access_block {
        Some(pab) => {
            let body = xml::xml_doc(&xml::wrap_ns(
                "PublicAccessBlockConfiguration",
                "http://s3.amazonaws.com/doc/2006-03-01/",
                &format!(
                    "{}{}{}{}",
                    xml::el("BlockPublicAcls", &pab.block_public_acls.to_string()),
                    xml::el("IgnorePublicAcls", &pab.ignore_public_acls.to_string()),
                    xml::el("BlockPublicPolicy", &pab.block_public_policy.to_string()),
                    xml::el("RestrictPublicBuckets", &pab.restrict_public_buckets.to_string()),
                ),
            ));
            (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
        }
        None => {
            (
                StatusCode::NOT_FOUND,
                xml::error_xml("NoSuchPublicAccessBlockConfiguration", ""),
            )
                .into_response()
        }
    }
}

async fn put_public_access_block(bucket: &str, state: SharedState, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let pab = PublicAccessBlock {
        block_public_acls: extract_xml_bool(&text, "BlockPublicAcls"),
        ignore_public_acls: extract_xml_bool(&text, "IgnorePublicAcls"),
        block_public_policy: extract_xml_bool(&text, "BlockPublicPolicy"),
        restrict_public_buckets: extract_xml_bool(&text, "RestrictPublicBuckets"),
    };

    let mut st = state.write().await;
    if let Some(b) = st.buckets.get_mut(bucket) {
        b.public_access_block = Some(pab);
        StatusCode::OK.into_response()
    } else {
        (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response()
    }
}

async fn delete_public_access_block(bucket: &str, state: SharedState) -> Response {
    let mut st = state.write().await;
    if let Some(b) = st.buckets.get_mut(bucket) {
        b.public_access_block = None;
    }
    StatusCode::NO_CONTENT.into_response()
}

// ── Bucket Policy ──

async fn get_bucket_policy(bucket: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let b = match st.buckets.get(bucket) {
        Some(b) => b,
        None => return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response(),
    };

    match &b.policy {
        Some(policy) => (StatusCode::OK, [("content-type", "application/json")], policy.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucketPolicy", "")).into_response(),
    }
}

async fn put_bucket_policy(bucket: &str, state: SharedState, body: Bytes) -> Response {
    let policy = String::from_utf8_lossy(&body).to_string();
    let mut st = state.write().await;
    if let Some(b) = st.buckets.get_mut(bucket) {
        b.policy = Some(policy);
        StatusCode::OK.into_response()
    } else {
        (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response()
    }
}

async fn delete_bucket_policy(bucket: &str, state: SharedState) -> Response {
    let mut st = state.write().await;
    if let Some(b) = st.buckets.get_mut(bucket) {
        b.policy = None;
    }
    StatusCode::NO_CONTENT.into_response()
}

// ── ListObjectsV2 ──

async fn list_objects_v2(bucket: &str, query: &str, state: SharedState) -> Response {
    let st = state.read().await;
    if !st.buckets.contains_key(bucket) {
        return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response();
    }

    let prefix = extract_query_param(query, "prefix").unwrap_or_default();
    let objects = st.list_keys(bucket, &prefix);

    let mut contents = String::new();
    for (key, ver) in &objects {
        contents.push_str(&xml::wrap(
            "Contents",
            &format!(
                "{}{}{}",
                xml::el("Key", key),
                xml::el("Size", &ver.body.len().to_string()),
                xml::el("LastModified", &ver.last_modified),
            ),
        ));
    }

    let body = xml::xml_doc(&xml::wrap_ns(
        "ListBucketResult",
        "http://s3.amazonaws.com/doc/2006-03-01/",
        &format!(
            "{}{}{}{}{}",
            xml::el("Name", bucket),
            xml::el("Prefix", &prefix),
            xml::el("KeyCount", &objects.len().to_string()),
            xml::el("IsTruncated", "false"),
            contents,
        ),
    ));

    (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
}

// ── ListObjectVersions ──

async fn list_object_versions(bucket: &str, query: &str, state: SharedState) -> Response {
    let st = state.read().await;
    if !st.buckets.contains_key(bucket) {
        return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response();
    }

    let prefix = extract_query_param(query, "prefix").unwrap_or_default();
    let mut versions_xml = String::new();
    let mut delete_markers_xml = String::new();

    for ((b, k), versions) in &st.objects {
        if b != bucket || !k.starts_with(&prefix) {
            continue;
        }
        let latest_idx = versions.len().saturating_sub(1);
        for (i, ver) in versions.iter().enumerate() {
            let is_latest = i == latest_idx;
            if ver.is_delete_marker {
                delete_markers_xml.push_str(&xml::wrap(
                    "DeleteMarker",
                    &format!(
                        "{}{}{}{}",
                        xml::el("Key", k),
                        xml::el("VersionId", &ver.version_id),
                        xml::el("IsLatest", &is_latest.to_string()),
                        xml::el("LastModified", &ver.last_modified),
                    ),
                ));
            } else {
                versions_xml.push_str(&xml::wrap(
                    "Version",
                    &format!(
                        "{}{}{}{}{}{}",
                        xml::el("Key", k),
                        xml::el("VersionId", &ver.version_id),
                        xml::el("IsLatest", &is_latest.to_string()),
                        xml::el("LastModified", &ver.last_modified),
                        xml::el("ETag", &format!("\"{}\"", ver.etag)),
                        xml::el("Size", &ver.body.len().to_string()),
                    ),
                ));
            }
        }
    }

    let body = xml::xml_doc(&xml::wrap_ns(
        "ListVersionsResult",
        "http://s3.amazonaws.com/doc/2006-03-01/",
        &format!(
            "{}{}{}{}{}",
            xml::el("Name", bucket),
            xml::el("Prefix", &prefix),
            xml::el("IsTruncated", "false"),
            versions_xml,
            delete_markers_xml,
        ),
    ));

    (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
}

// ── Object operations ──

async fn head_object(bucket: &str, key: &str, query: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let version = if let Some(vid) = extract_query_param(query, "versionId") {
        st.get_object_version(bucket, key, &vid)
    } else {
        st.get_latest_object(bucket, key)
    };

    match version {
        Some(ver) => {
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header("etag", format!("\"{}\"", ver.etag))
                .header("content-type", &ver.content_type)
                .header("content-length", ver.body.len().to_string())
                .header("last-modified", &ver.last_modified);
            if !ver.version_id.is_empty() {
                builder = builder.header("x-amz-version-id", &ver.version_id);
            }
            builder.body(Body::empty()).unwrap()
        }
        None => (StatusCode::NOT_FOUND, xml::error_xml("NoSuchKey", key)).into_response(),
    }
}

async fn get_object(bucket: &str, key: &str, query: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let version = if let Some(vid) = extract_query_param(query, "versionId") {
        st.get_object_version(bucket, key, &vid)
    } else {
        st.get_latest_object(bucket, key)
    };

    match version {
        Some(ver) => {
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header("etag", format!("\"{}\"", ver.etag))
                .header("content-type", &ver.content_type)
                .header("last-modified", &ver.last_modified);
            if !ver.version_id.is_empty() {
                builder = builder.header("x-amz-version-id", &ver.version_id);
            }
            builder.body(Body::from(ver.body.clone())).unwrap()
        }
        None => (StatusCode::NOT_FOUND, xml::error_xml("NoSuchKey", key)).into_response(),
    }
}

async fn put_object(
    bucket: &str,
    key: &str,
    headers: &HeaderMap,
    state: SharedState,
    body: Bytes,
) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let etag = format!("{:x}", md5_hash(&body));
    let now = jiff::Timestamp::now().to_string();

    let mut st = state.write().await;

    // Check if bucket exists
    let versioning = match st.buckets.get(bucket) {
        Some(b) => b.versioning.clone(),
        None => return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response(),
    };

    let version_id = if versioning == VersioningStatus::Enabled {
        Uuid::new_v4().to_string()
    } else {
        "null".to_string()
    };

    let obj = ObjectVersion {
        version_id: version_id.clone(),
        body,
        content_type,
        etag: etag.clone(),
        last_modified: now,
        is_delete_marker: false,
    };

    let versions = st
        .objects
        .entry((bucket.to_string(), key.to_string()))
        .or_default();

    if versioning == VersioningStatus::Enabled {
        versions.push(obj);
    } else {
        // Replace all versions with the new one
        versions.clear();
        versions.push(obj);
    }

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("etag", format!("\"{etag}\""));
    if version_id != "null" {
        builder = builder.header("x-amz-version-id", &version_id);
    }
    builder.body(Body::empty()).unwrap()
}

async fn delete_object(bucket: &str, key: &str, state: SharedState) -> Response {
    let mut st = state.write().await;

    let versioning = match st.buckets.get(bucket) {
        Some(b) => b.versioning.clone(),
        None => return StatusCode::NO_CONTENT.into_response(),
    };

    if versioning == VersioningStatus::Enabled {
        // Insert a delete marker
        let marker = ObjectVersion {
            version_id: Uuid::new_v4().to_string(),
            body: Bytes::new(),
            content_type: String::new(),
            etag: String::new(),
            last_modified: jiff::Timestamp::now().to_string(),
            is_delete_marker: true,
        };
        st.objects
            .entry((bucket.to_string(), key.to_string()))
            .or_default()
            .push(marker);
    } else {
        st.objects.remove(&(bucket.to_string(), key.to_string()));
    }

    StatusCode::NO_CONTENT.into_response()
}

// ── Helpers ──

/// Naive XML value extractor: finds `<Tag>value</Tag>`.
fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

fn extract_xml_bool(xml: &str, tag: &str) -> bool {
    extract_xml_value(xml, tag)
        .map(|v| v == "true")
        .unwrap_or(false)
}

fn extract_query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(
                    percent_encoding::percent_decode_str(v)
                        .decode_utf8_lossy()
                        .to_string(),
                );
            }
        }
    }
    None
}

/// Simple non-cryptographic hash for ETags.
fn md5_hash(data: &[u8]) -> u128 {
    // Use a simple FNV-like hash for ETag generation.
    // Real S3 uses MD5 but we just need unique deterministic strings.
    let mut hash: u128 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u128;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
