mod helpers;

use axum::http::{Method, StatusCode};

use helpers::{app, request};

async fn sts_request(
    app: &axum::Router,
    action: &str,
    extra_params: &str,
) -> helpers::TestResponse {
    let body = if extra_params.is_empty() {
        format!("Action={action}")
    } else {
        format!("Action={action}&{extra_params}")
    };
    request(app, Method::POST, "/?Action=placeholder", body).await
}

#[tokio::test]
async fn get_caller_identity_returns_default_root() {
    let app = app();
    let r = sts_request(&app, "GetCallerIdentity", "").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<Account>123456789012</Account>"));
    assert!(r.body.contains("root"));
}

#[tokio::test]
async fn assume_role_returns_temporary_credentials() {
    let app = app();
    let params =
        "RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2FTestRole&RoleSessionName=test-session";
    let r = sts_request(&app, "AssumeRole", params).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<AccessKeyId>"));
    assert!(r.body.contains("<SecretAccessKey>"));
    assert!(r.body.contains("<SessionToken>"));
    assert!(r.body.contains("<Expiration>"));
    assert!(r.body.contains("assumed-role"));
}

#[tokio::test]
async fn get_caller_identity_reflects_scenario_identity() {
    let app = {
        let state = claria_mock_aws::state::new_shared_state();
        {
            let mut st = state.write().await;
            claria_mock_aws::scenarios::load("bootstrapped", &mut st).unwrap();
        }
        claria_mock_aws::router::build_router(state)
    };

    let r = sts_request(&app, "GetCallerIdentity", "").await;
    assert_eq!(r.status, StatusCode::OK);
    // Bootstrapped scenario sets caller as claria-admin, not root
    assert!(r.body.contains("claria-admin"));
}
