use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("token expired")]
    TokenExpired,

    #[error("invalid token: {0}")]
    InvalidToken(String),

    #[error("user not found: {0}")]
    UserNotFound(String),

    #[error("MFA required")]
    MfaRequired { session: String },

    #[error("MFA verification failed: {0}")]
    MfaFailed(String),

    #[error("Cognito error: {0}")]
    Cognito(String),

    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("AWS config error: {0}")]
    Config(String),
}
