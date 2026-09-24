use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::user::UserRole;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// user id
    pub sub: String,
    pub role: UserRole,
    pub name: String,
    pub email: String,
    pub exp: i64,
    pub iat: i64,
}

pub fn issue_token(
    secret: &str,
    expiry_hours: i64,
    user_id: &str,
    role: UserRole,
    name: &str,
    email: &str,
) -> AppResult<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        role,
        name: name.to_string(),
        email: email.to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::hours(expiry_hours)).timestamp(),
    };
    // Mapped explicitly to Internal rather than letting `?` fall through to
    // AppError::Jwt: that variant means "the caller's token is bad" (401),
    // but a failure here is the server failing to mint a token for a caller
    // who just authenticated successfully -- their fault, wrong status.
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to issue auth token: {e}")))?;
    Ok(token)
}

pub fn decode_token(secret: &str, token: &str) -> AppResult<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}
