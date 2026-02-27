use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

/// JWT validation middleware.
///
/// Extracts the `Authorization: Bearer <token>` header and validates the JWT.
/// On success, inserts `AuthUser` into request extensions for handlers to use.
///
/// Full Cognito JWKS validation will be wired up when the decoding key
/// is added to AppState. For now, the token is extracted but not
/// cryptographically verified.
#[allow(dead_code)]
pub async fn require_auth(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    let sub = {
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        if token.is_empty() {
            return Err(StatusCode::UNAUTHORIZED);
        }

        // TODO: validate JWT against Cognito JWKS using claria_auth::jwt::validate_token
        token.to_string()
    };

    req.extensions_mut().insert(AuthUser { sub });

    Ok(next.run(req).await)
}

/// Authenticated user extracted from JWT claims.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct AuthUser {
    pub sub: String,
}
