use axum::{http::StatusCode, response::{IntoResponse, Response}};
use uuid::Uuid;

use crate::{params, state::SharedState, xml};

/// Dispatch STS actions from form-encoded POST body.
pub async fn dispatch(action: &str, _params: &str, state: SharedState) -> Response {
    match action {
        "GetCallerIdentity" => get_caller_identity(state).await,
        "AssumeRole" => assume_role(_params, state).await,
        _ => (
            StatusCode::BAD_REQUEST,
            xml::error_xml("InvalidAction", &format!("Unknown STS action: {action}")),
        )
            .into_response(),
    }
}

async fn get_caller_identity(state: SharedState) -> Response {
    let st = state.read().await;
    let id = &st.caller_identity;
    let body = xml::xml_doc(&xml::wrap(
        "GetCallerIdentityResponse",
        &xml::wrap(
            "GetCallerIdentityResult",
            &format!(
                "{}{}{}",
                xml::el("Account", &id.account),
                xml::el("Arn", &id.arn),
                xml::el("UserId", &id.user_id),
            ),
        ),
    ));
    (StatusCode::OK, [("content-type", "text/xml")], body).into_response()
}

async fn assume_role(params: &str, state: SharedState) -> Response {
    let st = state.read().await;
    let role_arn = extract_form_param(params, "RoleArn").unwrap_or_default();
    let session_name = extract_form_param(params, "RoleSessionName")
        .unwrap_or_else(|| "mock-session".to_string());

    // Extract account from role ARN: arn:aws:iam::ACCOUNT:role/NAME
    let account = role_arn
        .split(':')
        .nth(4)
        .unwrap_or(&st.caller_identity.account)
        .to_string();

    let access_key = format!("ASIA{}", Uuid::new_v4().to_string()[..16].to_uppercase());
    let secret_key = Uuid::new_v4().to_string();
    let session_token = format!("FwoGZX...mock-session-token...{}", Uuid::new_v4());
    let expiration = (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1)).to_string();
    let assumed_arn = format!(
        "arn:aws:sts::{account}:assumed-role/{session_name}"
    );

    let body = xml::xml_doc(&xml::wrap(
        "AssumeRoleResponse",
        &xml::wrap(
            "AssumeRoleResult",
            &format!(
                "{}{}",
                xml::wrap(
                    "Credentials",
                    &format!(
                        "{}{}{}{}",
                        xml::el("AccessKeyId", &access_key),
                        xml::el("SecretAccessKey", &secret_key),
                        xml::el("SessionToken", &session_token),
                        xml::el("Expiration", &expiration),
                    ),
                ),
                xml::wrap(
                    "AssumedRoleUser",
                    &format!(
                        "{}{}",
                        xml::el("Arn", &assumed_arn),
                        xml::el("AssumedRoleId", &format!("AROA{}:{session_name}", Uuid::new_v4().to_string()[..16].to_uppercase())),
                    ),
                ),
            ),
        ),
    ));

    (StatusCode::OK, [("content-type", "text/xml")], body).into_response()
}

fn extract_form_param(p: &str, key: &str) -> Option<String> {
    params::extract(p, key)
}
