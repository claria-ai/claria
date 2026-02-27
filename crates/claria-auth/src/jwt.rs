use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::error::AuthError;

/// Claims extracted from a Cognito JWT.
#[derive(Debug, Deserialize)]
pub struct CognitoClaims {
    pub sub: String,
    pub iss: String,
    pub token_use: String,
    pub exp: u64,
    pub iat: u64,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

/// Validate a Cognito JWT token.
///
/// In production, you would fetch the JWKS from the Cognito user pool
/// and use the matching key. This function takes a pre-fetched public key.
pub fn validate_token(
    token: &str,
    decoding_key: &DecodingKey,
    user_pool_id: &str,
    region: &str,
) -> Result<CognitoClaims, AuthError> {
    let issuer = format!("https://cognito-idp.{region}.amazonaws.com/{user_pool_id}");

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[&issuer]);
    validation.validate_exp = true;

    let token_data = decode::<CognitoClaims>(token, decoding_key, &validation)?;

    // Verify token_use is "access" or "id"
    let token_use = &token_data.claims.token_use;
    if token_use != "access" && token_use != "id" {
        return Err(AuthError::InvalidToken(format!(
            "unexpected token_use: {token_use}"
        )));
    }

    Ok(token_data.claims)
}
