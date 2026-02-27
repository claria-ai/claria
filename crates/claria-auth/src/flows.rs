use std::collections::HashMap;

use aws_sdk_cognitoidentityprovider::Client;
use aws_sdk_cognitoidentityprovider::types::AuthFlowType;
use tracing::info;

use crate::error::AuthError;

/// Result of an initial authentication attempt.
pub enum AuthResult {
    /// Authentication succeeded, tokens returned.
    Success {
        access_token: String,
        id_token: String,
        refresh_token: String,
    },
    /// MFA challenge required — caller must provide TOTP code.
    MfaChallenge { session: String },
}

/// Initiate username/password authentication with Cognito.
pub async fn initiate_auth(
    client: &Client,
    user_pool_client_id: &str,
    username: &str,
    password: &str,
) -> Result<AuthResult, AuthError> {
    info!(username = username, "initiating auth");

    let mut auth_params = HashMap::new();
    auth_params.insert("USERNAME".to_string(), username.to_string());
    auth_params.insert("PASSWORD".to_string(), password.to_string());

    let resp = client
        .initiate_auth()
        .auth_flow(AuthFlowType::UserPasswordAuth)
        .client_id(user_pool_client_id)
        .set_auth_parameters(Some(auth_params))
        .send()
        .await
        .map_err(|e| AuthError::Cognito(e.into_service_error().to_string()))?;

    if let Some(result) = resp.authentication_result() {
        Ok(AuthResult::Success {
            access_token: result.access_token().unwrap_or_default().to_string(),
            id_token: result.id_token().unwrap_or_default().to_string(),
            refresh_token: result.refresh_token().unwrap_or_default().to_string(),
        })
    } else if resp.challenge_name().is_some() {
        let session = resp.session().unwrap_or_default().to_string();
        Ok(AuthResult::MfaChallenge { session })
    } else {
        Err(AuthError::AuthFailed("unexpected response".to_string()))
    }
}

/// Respond to an MFA challenge with a TOTP code.
pub async fn respond_to_mfa(
    client: &Client,
    user_pool_client_id: &str,
    username: &str,
    session: &str,
    mfa_code: &str,
) -> Result<AuthResult, AuthError> {
    info!(username = username, "responding to MFA challenge");

    let mut challenge_responses = HashMap::new();
    challenge_responses.insert("USERNAME".to_string(), username.to_string());
    challenge_responses.insert("SOFTWARE_TOKEN_MFA_CODE".to_string(), mfa_code.to_string());

    let resp = client
        .respond_to_auth_challenge()
        .client_id(user_pool_client_id)
        .challenge_name(aws_sdk_cognitoidentityprovider::types::ChallengeNameType::SoftwareTokenMfa)
        .set_challenge_responses(Some(challenge_responses))
        .session(session)
        .send()
        .await
        .map_err(|e| AuthError::MfaFailed(e.into_service_error().to_string()))?;

    if let Some(result) = resp.authentication_result() {
        Ok(AuthResult::Success {
            access_token: result.access_token().unwrap_or_default().to_string(),
            id_token: result.id_token().unwrap_or_default().to_string(),
            refresh_token: result.refresh_token().unwrap_or_default().to_string(),
        })
    } else {
        Err(AuthError::MfaFailed("MFA response did not return tokens".to_string()))
    }
}

/// Refresh tokens using a refresh token.
pub async fn refresh_auth(
    client: &Client,
    user_pool_client_id: &str,
    refresh_token: &str,
) -> Result<AuthResult, AuthError> {
    let mut auth_params = HashMap::new();
    auth_params.insert("REFRESH_TOKEN".to_string(), refresh_token.to_string());

    let resp = client
        .initiate_auth()
        .auth_flow(AuthFlowType::RefreshTokenAuth)
        .client_id(user_pool_client_id)
        .set_auth_parameters(Some(auth_params))
        .send()
        .await
        .map_err(|e| AuthError::Cognito(e.into_service_error().to_string()))?;

    if let Some(result) = resp.authentication_result() {
        Ok(AuthResult::Success {
            access_token: result.access_token().unwrap_or_default().to_string(),
            id_token: result.id_token().unwrap_or_default().to_string(),
            // Refresh token may not be returned on refresh
            refresh_token: result
                .refresh_token()
                .unwrap_or(refresh_token)
                .to_string(),
        })
    } else {
        Err(AuthError::AuthFailed("refresh failed".to_string()))
    }
}
