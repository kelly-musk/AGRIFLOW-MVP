use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::auth::jwt::decode_token;
use crate::error::{AppError, AppResult};
use crate::models::user::UserRole;
use crate::state::AppState;

/// The authenticated caller, derived from a verified JWT. Cheap to build per
/// request — trusts the signed claims rather than round-tripping to the DB,
/// since the token already carries id/role/name/email.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub role: UserRole,
    pub name: String,
    pub email: String,
}

impl AuthUser {
    pub fn require_role(&self, role: UserRole) -> AppResult<()> {
        if self.role != role {
            return Err(AppError::Forbidden(format!(
                "This action requires the '{role}' role."
            )));
        }
        Ok(())
    }

    /// Multi-role gating -- e.g. `wallet::summary`/`withdraw`, open to both
    /// Supplier and Logistics.
    pub fn require_any_role(&self, roles: &[UserRole]) -> AppResult<()> {
        if !roles.contains(&self.role) {
            return Err(AppError::Forbidden(
                "You are not permitted to perform this action.".to_string(),
            ));
        }
        Ok(())
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing Authorization header.".to_string()))?;

        let token = header.strip_prefix("Bearer ").ok_or_else(|| {
            AppError::Unauthorized("Authorization header must be a Bearer token.".to_string())
        })?;

        let claims = decode_token(&state.config.jwt_secret, token)?;

        Ok(AuthUser {
            user_id: claims.sub,
            role: claims.role,
            name: claims.name,
            email: claims.email,
        })
    }
}
