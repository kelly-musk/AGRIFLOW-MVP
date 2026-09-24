use axum::{Json, extract::State, http::HeaderMap};

use crate::auth::{AuthUser, jwt::issue_token, password::{hash_password, verify_password}};
use crate::error::{AppError, AppResult};
use crate::ids;
use crate::json_extractor::AppJson;
use crate::models::user::{AuthResponse, LoginRequest, RegisterRequest, User, UserPublic, UserRole};
use crate::state::AppState;

const ADMIN_KEY_HEADER: &str = "x-admin-registration-key";

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    AppJson(body): AppJson<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    if body.name.trim().is_empty() || body.email.trim().is_empty() {
        return Err(AppError::BadRequest("Name and email are required.".into()));
    }
    // Admins can't sign themselves up: registering one requires the
    // server-side ADMIN_REGISTRATION_KEY (used by seed scripts and tests).
    if body.role == UserRole::Admin {
        let presented = headers.get(ADMIN_KEY_HEADER).and_then(|v| v.to_str().ok());
        let authorized = match (&state.config.admin_registration_key, presented) {
            (Some(expected), Some(presented)) => constant_time_eq(expected, presented),
            _ => false,
        };
        if !authorized {
            return Err(AppError::Forbidden(
                "Admin accounts can't be self-registered.".into(),
            ));
        }
    }
    if body.password.len() < 6 {
        return Err(AppError::BadRequest(
            "Password must be at least 6 characters.".into(),
        ));
    }

    let existing = sqlx::query_scalar!(
        "SELECT id FROM users WHERE lower(email) = lower($1)",
        body.email
    )
    .fetch_optional(&state.db)
    .await?;
    if existing.is_some() {
        return Err(AppError::Conflict(
            "An account with this email already exists.".into(),
        ));
    }

    let id = ids::user_id(&body.role.to_string());
    let password_hash = hash_password(&body.password)?;
    let org_name = body.organization_name.clone().or_else(|| Some(body.name.clone()));

    let user = sqlx::query_as!(
        User,
        r#"
        INSERT INTO users (id, email, password_hash, name, role, organization_name, phone, location, verified, profile_complete)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, TRUE)
        RETURNING id, email, password_hash, name, role as "role: _", organization_name, phone, location, verified, profile_complete, created_at, updated_at
        "#,
        id,
        body.email,
        password_hash,
        body.name,
        body.role as _,
        org_name,
        body.phone,
        body.location,
    )
    .fetch_one(&state.db)
    .await?;

    let token = issue_token(
        &state.config.jwt_secret,
        state.config.jwt_expiry_hours,
        &user.id,
        user.role,
        &user.name,
        &user.email,
    )?;

    state.mailer.send_welcome(&user.email, &user.name, user.role);

    Ok(Json(AuthResponse {
        token,
        user: UserPublic::from(user),
    }))
}

/// Compares secrets without short-circuiting on the first differing byte,
/// so response timing doesn't leak how much of the key a guess got right.
fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn login(
    State(state): State<AppState>,
    AppJson(body): AppJson<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    let user = sqlx::query_as!(
        User,
        r#"
        SELECT id, email, password_hash, name, role as "role: _", organization_name, phone, location, verified, profile_complete, created_at, updated_at
        FROM users WHERE lower(email) = lower($1)
        "#,
        body.email
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid email or password.".into()))?;

    if !verify_password(&user.password_hash, &body.password)? {
        return Err(AppError::Unauthorized("Invalid email or password.".into()));
    }

    let token = issue_token(
        &state.config.jwt_secret,
        state.config.jwt_expiry_hours,
        &user.id,
        user.role,
        &user.name,
        &user.email,
    )?;

    Ok(Json(AuthResponse {
        token,
        user: UserPublic::from(user),
    }))
}

pub async fn me(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<UserPublic>> {
    let user = sqlx::query_as!(
        User,
        r#"
        SELECT id, email, password_hash, name, role as "role: _", organization_name, phone, location, verified, profile_complete, created_at, updated_at
        FROM users WHERE id = $1
        "#,
        auth.user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("User not found.".into()))?;

    Ok(Json(UserPublic::from(user)))
}
