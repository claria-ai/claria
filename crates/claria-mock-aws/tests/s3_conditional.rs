mod helpers;

use axum::http::{Method, StatusCode};
use helpers::{app, request, request_with_header};

#[tokio::test]
async fn if_none_match_star_prevents_overwrite() {
    let app = app();
    request(&app, Method::PUT, "/conditional", "").await;
    let first = request_with_header(
        &app,
        Method::PUT,
        "/conditional/workspace.json",
        "if-none-match",
        "*",
        "first",
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);

    let second = request_with_header(
        &app,
        Method::PUT,
        "/conditional/workspace.json",
        "if-none-match",
        "*",
        "second",
    )
    .await;
    assert_eq!(second.status, StatusCode::PRECONDITION_FAILED);
    assert!(second.body.contains("PreconditionFailed"));

    let current = request(&app, Method::GET, "/conditional/workspace.json", "").await;
    assert_eq!(current.body, "first");
}

#[tokio::test]
async fn if_match_accepts_current_etag_and_rejects_stale_etag() {
    let app = app();
    request(&app, Method::PUT, "/conditional-match", "").await;
    let first = request(
        &app,
        Method::PUT,
        "/conditional-match/workspace.json",
        "revision zero",
    )
    .await;
    let first_etag = first.header("etag").expect("etag").to_string();

    let updated = request_with_header(
        &app,
        Method::PUT,
        "/conditional-match/workspace.json",
        "if-match",
        &first_etag,
        "revision one",
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_ne!(updated.header("etag"), Some(first_etag.as_str()));

    let stale = request_with_header(
        &app,
        Method::PUT,
        "/conditional-match/workspace.json",
        "if-match",
        &first_etag,
        "stale revision two",
    )
    .await;
    assert_eq!(stale.status, StatusCode::PRECONDITION_FAILED);

    let current = request(&app, Method::GET, "/conditional-match/workspace.json", "").await;
    assert_eq!(current.body, "revision one");
}
