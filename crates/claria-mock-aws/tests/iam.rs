mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request};

fn iam_post(action: &str, extra_params: &str) -> String {
    if extra_params.is_empty() {
        format!("Action={action}")
    } else {
        format!("Action={action}&{extra_params}")
    }
}

async fn iam_request(
    app: &axum::Router,
    action: &str,
    extra_params: &str,
) -> helpers::TestResponse {
    let body = iam_post(action, extra_params);
    request(app, Method::POST, "/?Action=placeholder", body).await
}

// ── User CRUD ───────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_get_user() {
    let app = app();
    let r = iam_request(&app, "CreateUser", "UserName=testuser").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("arn:aws:iam::"));
    assert!(r.body.contains("testuser"));

    let r = iam_request(&app, "GetUser", "UserName=testuser").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<UserName>testuser</UserName>"));
}

#[tokio::test]
async fn get_nonexistent_user_returns_404() {
    let app = app();
    let r = iam_request(&app, "GetUser", "UserName=ghost").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("NoSuchEntity"));
}

#[tokio::test]
async fn create_duplicate_user_returns_409() {
    let app = app();
    iam_request(&app, "CreateUser", "UserName=dup").await;
    let r = iam_request(&app, "CreateUser", "UserName=dup").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.body.contains("EntityAlreadyExists"));
}

#[tokio::test]
async fn list_users() {
    let app = app();
    iam_request(&app, "CreateUser", "UserName=alice").await;
    iam_request(&app, "CreateUser", "UserName=bob").await;

    let r = iam_request(&app, "ListUsers", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<UserName>alice</UserName>"));
    assert!(r.body.contains("<UserName>bob</UserName>"));
}

// ── Managed Policy CRUD ─────────────────────────────────────────────

#[tokio::test]
async fn create_and_get_policy() {
    let app = app();
    let doc = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let params = format!(
        "PolicyName=TestPolicy&PolicyDocument={}&Description=test+desc",
        percent_encode(doc)
    );
    let r = iam_request(&app, "CreatePolicy", &params).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("TestPolicy")); // ARN contains policy name

    // Extract ARN from response for GetPolicy
    let arn = extract_xml_value(&r.body, "Arn").unwrap();
    let r = iam_request(
        &app,
        "GetPolicy",
        &format!("PolicyArn={}", percent_encode(&arn)),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<PolicyName>TestPolicy</PolicyName>"));
}

#[tokio::test]
async fn get_nonexistent_policy_returns_404() {
    let app = app();
    let r = iam_request(
        &app,
        "GetPolicy",
        "PolicyArn=arn%3Aaws%3Aiam%3A%3A123%3Apolicy%2FNope",
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ── Policy Versions ─────────────────────────────────────────────────

#[tokio::test]
async fn create_and_list_policy_versions() {
    let app = app();
    let doc = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let params = format!("PolicyName=VerPol&PolicyDocument={}", percent_encode(doc));
    let r = iam_request(&app, "CreatePolicy", &params).await;
    let arn = extract_xml_value(&r.body, "Arn").unwrap();

    let doc2 = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow"}]}"#;
    let params = format!(
        "PolicyArn={}&PolicyDocument={}&SetAsDefault=true",
        percent_encode(&arn),
        percent_encode(doc2)
    );
    let r = iam_request(&app, "CreatePolicyVersion", &params).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<VersionId>v2</VersionId>"));

    let r = iam_request(
        &app,
        "ListPolicyVersions",
        &format!("PolicyArn={}", percent_encode(&arn)),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("v1"));
    assert!(r.body.contains("v2"));
}

#[tokio::test]
async fn delete_policy_version() {
    let app = app();
    let doc = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let params = format!(
        "PolicyName=DelVerPol&PolicyDocument={}",
        percent_encode(doc)
    );
    let r = iam_request(&app, "CreatePolicy", &params).await;
    let arn = extract_xml_value(&r.body, "Arn").unwrap();

    // Create v2
    let params = format!(
        "PolicyArn={}&PolicyDocument={}&SetAsDefault=false",
        percent_encode(&arn),
        percent_encode(doc)
    );
    iam_request(&app, "CreatePolicyVersion", &params).await;

    // Delete v2
    let params = format!("PolicyArn={}&VersionId=v2", percent_encode(&arn));
    let r = iam_request(&app, "DeletePolicyVersion", &params).await;
    assert_eq!(r.status, StatusCode::OK);

    // Should only have v1
    let r = iam_request(
        &app,
        "ListPolicyVersions",
        &format!("PolicyArn={}", percent_encode(&arn)),
    )
    .await;
    assert!(r.body.contains("v1"));
    assert!(!r.body.contains("v2"));
}

// ── Attach / Detach ─────────────────────────────────────────────────

#[tokio::test]
async fn attach_and_list_user_policies() {
    let app = app();
    iam_request(&app, "CreateUser", "UserName=poluser").await;

    let doc = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let r = iam_request(
        &app,
        "CreatePolicy",
        &format!(
            "PolicyName=AttachPol&PolicyDocument={}",
            percent_encode(doc)
        ),
    )
    .await;
    let arn = extract_xml_value(&r.body, "Arn").unwrap();

    let params = format!("UserName=poluser&PolicyArn={}", percent_encode(&arn));
    let r = iam_request(&app, "AttachUserPolicy", &params).await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(&app, "ListAttachedUserPolicies", "UserName=poluser").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("AttachPol"));

    // Detach
    let r = iam_request(&app, "DetachUserPolicy", &params).await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(&app, "ListAttachedUserPolicies", "UserName=poluser").await;
    assert!(!r.body.contains("AttachPol"));
}

// ── Inline Policies ─────────────────────────────────────────────────

#[tokio::test]
async fn put_get_delete_inline_policy() {
    let app = app();
    iam_request(&app, "CreateUser", "UserName=inluser").await;

    let doc = r#"{"Version":"2012-10-17","Statement":[]}"#;
    let params = format!(
        "UserName=inluser&PolicyName=InlineOne&PolicyDocument={}",
        percent_encode(doc)
    );
    let r = iam_request(&app, "PutUserPolicy", &params).await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(
        &app,
        "GetUserPolicy",
        "UserName=inluser&PolicyName=InlineOne",
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("InlineOne"));

    let r = iam_request(
        &app,
        "DeleteUserPolicy",
        "UserName=inluser&PolicyName=InlineOne",
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(
        &app,
        "GetUserPolicy",
        "UserName=inluser&PolicyName=InlineOne",
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ── Access Keys ─────────────────────────────────────────────────────

#[tokio::test]
async fn create_list_delete_access_key() {
    let app = app();
    iam_request(&app, "CreateUser", "UserName=keyuser").await;

    let r = iam_request(&app, "CreateAccessKey", "UserName=keyuser").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<AccessKeyId>"));
    assert!(r.body.contains("<SecretAccessKey>"));
    let access_key_id = extract_xml_value(&r.body, "AccessKeyId").unwrap();

    let r = iam_request(&app, "ListAccessKeys", "UserName=keyuser").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(&access_key_id));

    let r = iam_request(
        &app,
        "GetAccessKeyLastUsed",
        &format!("AccessKeyId={access_key_id}"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(
        &app,
        "DeleteAccessKey",
        &format!("UserName=keyuser&AccessKeyId={access_key_id}"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(&app, "ListAccessKeys", "UserName=keyuser").await;
    assert!(!r.body.contains(&access_key_id));
}

#[tokio::test]
async fn access_key_limit_is_two() {
    let app = app();
    iam_request(&app, "CreateUser", "UserName=limituser").await;

    let r = iam_request(&app, "CreateAccessKey", "UserName=limituser").await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(&app, "CreateAccessKey", "UserName=limituser").await;
    assert_eq!(r.status, StatusCode::OK);

    let r = iam_request(&app, "CreateAccessKey", "UserName=limituser").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.body.contains("LimitExceeded"));
}

// ── Helpers ─────────────────────────────────────────────────────────

fn percent_encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}
