use axum::{
    body::Body,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use uuid::Uuid;

use crate::{
    params,
    state::{BucketState, ObjectVersion, PublicAccessBlock, SharedState, VersioningStatus},
    xml,
};

/// Page size ceiling for ListObjectVersions, matching S3's own cap.
const MAX_LIST_KEYS: usize = 1000;

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
    // Route by query param presence. `?delete` is matched as a whole flag,
    // not a substring: a ListObjectsV2 prefix is free to contain the word.
    if has_flag(query, "delete") {
        return match *method {
            Method::POST => delete_objects(bucket, state, body).await,
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        };
    }
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
        Method::DELETE => delete_object(bucket, &key, query, state).await,
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

// ── Bucket operations ──

async fn head_bucket(bucket: &str, state: SharedState) -> Response {
    let st = state.read().await;
    if st.buckets.contains_key(bucket) {
        StatusCode::OK.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            xml::error_xml("NoSuchBucket", "The specified bucket does not exist"),
        )
            .into_response()
    }
}

async fn create_bucket(bucket: &str, state: SharedState, body: Bytes) -> Response {
    let region = if body.is_empty() {
        "us-east-1".to_string()
    } else {
        // Parse <CreateBucketConfiguration><LocationConstraint>REGION</...>
        let text = String::from_utf8_lossy(&body);
        extract_xml_value(&text, "LocationConstraint").unwrap_or_else(|| "us-east-1".to_string())
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

    // S3 refuses to drop a bucket that still holds anything — and on a
    // versioning-enabled bucket a delete marker counts, which is exactly how
    // "I deleted every object" still leaves a bucket that won't go away.
    let residue = st
        .objects
        .iter()
        .any(|((b, _), versions)| b == bucket && !versions.is_empty());
    if residue {
        return (
            StatusCode::CONFLICT,
            xml::error_xml(
                "BucketNotEmpty",
                "The bucket you tried to delete is not empty. You must delete all versions in the bucket.",
            ),
        )
            .into_response();
    }

    st.buckets.remove(bucket);
    st.objects.retain(|(b, _), _| b != bucket);
    StatusCode::NO_CONTENT.into_response()
}

// ── DeleteObjects (batch) ──

/// `POST /{bucket}?delete` — delete up to 1,000 objects or object versions.
///
/// Answers 200 with a body that mixes `Deleted` and `Error` entries: a failure
/// here is not an HTTP error, which is the trap this mock exists to reproduce.
async fn delete_objects(bucket: &str, state: SharedState, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let targets = parse_delete_request(&text);

    let mut st = state.write().await;

    let versioning = match st.buckets.get(bucket) {
        Some(b) => b.versioning.clone(),
        None => return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response(),
    };

    st.s3_delete_objects_batches.push(targets.len());

    let mut results = String::new();

    for (key, version_id) in targets {
        if st.s3_delete_object_failures.contains(&key) {
            results.push_str(&xml::wrap(
                "Error",
                &format!(
                    "{}{}{}",
                    xml::el("Key", &key),
                    xml::el("Code", "AccessDenied"),
                    xml::el("Message", "Access Denied"),
                ),
            ));
            continue;
        }

        let entry = (bucket.to_string(), key.clone());

        match &version_id {
            // A versioned delete removes that exact version, delete markers
            // included, and never leaves a new marker behind.
            Some(vid) => {
                if let Some(versions) = st.objects.get_mut(&entry) {
                    versions.retain(|v| &v.version_id != vid);
                    if versions.is_empty() {
                        st.objects.remove(&entry);
                    }
                }
                results.push_str(&xml::wrap(
                    "Deleted",
                    &format!("{}{}", xml::el("Key", &key), xml::el("VersionId", vid)),
                ));
            }
            None if versioning == VersioningStatus::Enabled => {
                let marker_id = Uuid::new_v4().to_string();
                st.objects.entry(entry).or_default().push(ObjectVersion {
                    version_id: marker_id.clone(),
                    body: Bytes::new(),
                    content_type: String::new(),
                    etag: String::new(),
                    last_modified: jiff::Timestamp::now().to_string(),
                    is_delete_marker: true,
                });
                results.push_str(&xml::wrap(
                    "Deleted",
                    &format!(
                        "{}{}{}",
                        xml::el("Key", &key),
                        xml::el("DeleteMarker", "true"),
                        xml::el("DeleteMarkerVersionId", &marker_id),
                    ),
                ));
            }
            None => {
                st.objects.remove(&entry);
                results.push_str(&xml::wrap("Deleted", &xml::el("Key", &key)));
            }
        }
    }

    let body = xml::xml_doc(&xml::wrap_ns(
        "DeleteResult",
        "http://s3.amazonaws.com/doc/2006-03-01/",
        &results,
    ));

    (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
}

/// Pull `(key, version_id)` pairs out of a `Delete` request body.
fn parse_delete_request(xml_body: &str) -> Vec<(String, Option<String>)> {
    let mut targets = Vec::new();
    let mut rest = xml_body;

    while let Some(start) = rest.find("<Object>") {
        let after_open = &rest[start + "<Object>".len()..];
        let Some(end) = after_open.find("</Object>") else {
            break;
        };
        let block = &after_open[..end];
        if let Some(key) = extract_xml_value(block, "Key") {
            targets.push((key, extract_xml_value(block, "VersionId")));
        }
        rest = &after_open[end..];
    }

    targets
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
        None => (
            StatusCode::NOT_FOUND,
            xml::error_xml(
                "ServerSideEncryptionConfigurationNotFoundError",
                "The server side encryption configuration was not found",
            ),
        )
            .into_response(),
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
                    xml::el(
                        "RestrictPublicBuckets",
                        &pab.restrict_public_buckets.to_string()
                    ),
                ),
            ));
            (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            xml::error_xml("NoSuchPublicAccessBlockConfiguration", ""),
        )
            .into_response(),
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
        Some(policy) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            policy.clone(),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            xml::error_xml("NoSuchBucketPolicy", ""),
        )
            .into_response(),
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
                "{}{}{}{}",
                xml::el("Key", key),
                xml::el("Size", &ver.body.len().to_string()),
                xml::el("LastModified", &ver.last_modified),
                xml::el("ETag", &format!("\"{}\"", ver.etag)),
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
    let max_keys = extract_query_param(query, "max-keys")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(MAX_LIST_KEYS)
        .min(MAX_LIST_KEYS);
    let key_marker = extract_query_param(query, "key-marker");
    let version_id_marker = extract_query_param(query, "version-id-marker");

    // S3 orders versions by key ascending, newest version first within a key.
    // State stores each version stack oldest-first, so walk it in reverse.
    let mut keys: Vec<&String> = st
        .objects
        .keys()
        .filter(|(b, k)| b == bucket && k.starts_with(&prefix))
        .map(|(_, k)| k)
        .collect();
    keys.sort_unstable();

    let mut entries: Vec<(&str, &ObjectVersion, bool)> = Vec::new();
    for key in keys {
        let versions = &st.objects[&(bucket.to_string(), key.clone())];
        for (i, ver) in versions.iter().rev().enumerate() {
            entries.push((key.as_str(), ver, i == 0));
        }
    }

    // Resume after the marker pair: a key marker alone starts at the next key.
    if let Some(km) = &key_marker {
        let start = match &version_id_marker {
            Some(vm) => entries
                .iter()
                .position(|(k, v, _)| k == km && &v.version_id == vm)
                .map(|i| i + 1)
                .unwrap_or(0),
            None => entries
                .iter()
                .position(|(k, _, _)| *k > km.as_str())
                .unwrap_or(entries.len()),
        };
        entries.drain(..start);
    }

    let truncated = entries.len() > max_keys;
    entries.truncate(max_keys);

    let next_markers = if truncated {
        entries
            .last()
            .map(|(k, v, _)| {
                format!(
                    "{}{}",
                    xml::el("NextKeyMarker", k),
                    xml::el("NextVersionIdMarker", &v.version_id),
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    let mut versions_xml = String::new();
    let mut delete_markers_xml = String::new();

    for (key, ver, is_latest) in &entries {
        if ver.is_delete_marker {
            delete_markers_xml.push_str(&xml::wrap(
                "DeleteMarker",
                &format!(
                    "{}{}{}{}",
                    xml::el("Key", key),
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
                    xml::el("Key", key),
                    xml::el("VersionId", &ver.version_id),
                    xml::el("IsLatest", &is_latest.to_string()),
                    xml::el("LastModified", &ver.last_modified),
                    xml::el("ETag", &format!("\"{}\"", ver.etag)),
                    xml::el("Size", &ver.body.len().to_string()),
                ),
            ));
        }
    }

    let body = xml::xml_doc(&xml::wrap_ns(
        "ListVersionsResult",
        "http://s3.amazonaws.com/doc/2006-03-01/",
        &format!(
            "{}{}{}{}{}{}{}",
            xml::el("Name", bucket),
            xml::el("Prefix", &prefix),
            xml::el("MaxKeys", &max_keys.to_string()),
            xml::el("IsTruncated", &truncated.to_string()),
            next_markers,
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
                .header("last-modified", http_date(&ver.last_modified));
            if !ver.version_id.is_empty() {
                builder = builder.header("x-amz-version-id", &ver.version_id);
            }
            builder.body(Body::empty()).unwrap()
        }
        None => (StatusCode::NOT_FOUND, xml::error_xml("NoSuchKey", key)).into_response(),
    }
}

async fn get_object(bucket: &str, key: &str, query: &str, state: SharedState) -> Response {
    let mut st = state.write().await;
    *st.s3_get_object_requests
        .entry(key.to_string())
        .or_default() += 1;
    if st.s3_get_object_failures.contains(key) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            xml::error_xml("InternalError", "Injected GET Object failure"),
        )
            .into_response();
    }
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
                .header("last-modified", http_date(&ver.last_modified));
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

    // Check if bucket exists.
    let versioning = match st.buckets.get(bucket) {
        Some(b) => b.versioning.clone(),
        None => return (StatusCode::NOT_FOUND, xml::error_xml("NoSuchBucket", "")).into_response(),
    };

    // Fault-inject the two conditional race responses documented by S3.
    // Hooks apply only to requests that actually carry a write precondition.
    let conditional = headers.contains_key("if-none-match") || headers.contains_key("if-match");
    if conditional {
        if let Some(remaining) = st.s3_conditional_conflicts_remaining.get_mut(key)
            && *remaining > 0
        {
            *remaining -= 1;
            return (
                StatusCode::CONFLICT,
                xml::error_xml(
                    "ConditionalRequestConflict",
                    "A conflicting conditional operation is in progress",
                ),
            )
                .into_response();
        }
        if let Some(remaining) = st.s3_precondition_failures_remaining.get_mut(key)
            && *remaining > 0
        {
            *remaining -= 1;
            return (
                StatusCode::PRECONDITION_FAILED,
                xml::error_xml(
                    "PreconditionFailed",
                    "At least one of the preconditions you specified did not hold",
                ),
            )
                .into_response();
        }
    }

    // Enforce the same conditional-write preconditions report workspaces use
    // against real S3. Header values carry quoted ETags on the wire while the
    // mock stores the bare digest.
    let current = st
        .objects
        .get(&(bucket.to_string(), key.to_string()))
        .and_then(|versions| versions.last())
        .filter(|version| !version.is_delete_marker);

    if headers
        .get("if-none-match")
        .and_then(|value| value.to_str().ok())
        == Some("*")
        && current.is_some()
    {
        return (
            StatusCode::PRECONDITION_FAILED,
            xml::error_xml(
                "PreconditionFailed",
                "At least one of the preconditions you specified did not hold",
            ),
        )
            .into_response();
    }

    if let Some(expected) = headers
        .get("if-match")
        .and_then(|value| value.to_str().ok())
    {
        let expected = expected.trim_matches('"');
        if current.is_none_or(|version| version.etag != expected) {
            return (
                StatusCode::PRECONDITION_FAILED,
                xml::error_xml(
                    "PreconditionFailed",
                    "At least one of the preconditions you specified did not hold",
                ),
            )
                .into_response();
        }
    }

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

async fn delete_object(bucket: &str, key: &str, query: &str, state: SharedState) -> Response {
    let mut st = state.write().await;

    let versioning = match st.buckets.get(bucket) {
        Some(b) => b.versioning.clone(),
        None => return StatusCode::NO_CONTENT.into_response(),
    };

    // A versioned delete removes that exact version rather than stacking a
    // delete marker on top of it.
    if let Some(vid) = extract_query_param(query, "versionId") {
        let entry = (bucket.to_string(), key.to_string());
        if let Some(versions) = st.objects.get_mut(&entry) {
            versions.retain(|v| v.version_id != vid);
            if versions.is_empty() {
                st.objects.remove(&entry);
            }
        }
        return StatusCode::NO_CONTENT.into_response();
    }

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
    params::extract(query, key)
}

/// Whether a valueless query flag (`?delete`, `?delete=`) is present.
fn has_flag(query: &str, flag: &str) -> bool {
    query
        .split('&')
        .any(|pair| pair == flag || pair.split_once('=').map(|(k, _)| k) == Some(flag))
}

/// Convert an ISO 8601 timestamp (jiff's default `Timestamp` Display, which is
/// what state stores) into RFC 7231 HTTP-date format. The AWS SDK strictly
/// parses `Last-Modified` as HTTP-date and rejects ISO 8601, so headers must
/// use this — XML payloads keep the ISO form. Falls back to the input string
/// if parsing fails so we never produce an empty header.
fn http_date(iso: &str) -> String {
    iso.parse::<jiff::Timestamp>()
        .ok()
        .map(|ts| {
            ts.to_zoned(jiff::tz::TimeZone::UTC)
                .strftime("%a, %d %b %Y %H:%M:%S GMT")
                .to_string()
        })
        .unwrap_or_else(|| iso.to_string())
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
