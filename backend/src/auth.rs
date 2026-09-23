use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    extract::FromRequestParts,
    http::request::Parts,
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::{error::AppError, state::AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,   // user id
    pub role: String,
    pub name: String,
    pub email: String,
    pub exp: usize,
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("password hashing failed: {e}")))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else { return false };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

pub fn issue_token(secret: &str, user_id: &str, role: &str, name: &str, email: &str) -> Result<String, AppError> {
    let exp = (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        role: role.to_string(),
        name: name.to_string(),
        email: email.to_string(),
        exp,
    };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(format!("token issuance failed: {e}")))
}

fn decode_token(secret: &str, token: &str) -> Result<Claims, AppError> {
    decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &Validation::default())
        .map(|d| d.claims)
        .map_err(|_| AppError::Unauthorized("Invalid or expired token.".to_string()))
}

/// Authenticated user extracted from the `Authorization: Bearer <jwt>` header.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub role: String,
    pub name: String,
    pub email: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing Authorization header.".to_string()))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("Authorization header must be a Bearer token.".to_string()))?;

        let claims = decode_token(&state.jwt_secret, token)?;
        Ok(AuthUser {
            id: claims.sub,
            role: claims.role,
            name: claims.name,
            email: claims.email,
        })
    }
}

impl AuthUser {
    pub fn require_role(&self, role: &str) -> Result<(), AppError> {
        if self.role != role {
            return Err(AppError::Forbidden(format!("This action requires the '{role}' role.")));
        }
        Ok(())
    }

    pub fn require_any_role(&self, roles: &[&str]) -> Result<(), AppError> {
        if !roles.contains(&self.role.as_str()) {
            return Err(AppError::Forbidden("You are not authorized to perform this action.".to_string()));
        }
        Ok(())
    }
}
