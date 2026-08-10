#![allow(dead_code)]

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use claria_mock_aws::{router, state};

pub fn app() -> Router {
    let state = state::new_shared_state();
    router::build_router(state)
}

pub async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    body: impl Into<Body>,
) -> TestResponse {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .body(body.into())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&body_bytes).to_string();
    TestResponse {
        status,
        body,
        headers,
    }
}

pub async fn request_with_header(
    app: &Router,
    method: Method,
    uri: &str,
    header_name: &str,
    header_value: &str,
    body: impl Into<Body>,
) -> TestResponse {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header_name, header_value)
        .body(body.into())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&body_bytes).to_string();
    TestResponse {
        status,
        body,
        headers,
    }
}

pub struct TestResponse {
    pub status: StatusCode,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

impl TestResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}
