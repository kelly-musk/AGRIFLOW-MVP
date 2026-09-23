use axum::{Json, extract::State};

use crate::{
    auth::{self, AuthUser},
    error::{AppError, AppResult},
    ids,
    models::{AuthSession, LoginRequest, RegisterRequest, UpdateProfileRequest, User},
    services::{self, AuditParams},
    state::AppState,
};

const VALID_ROLES: [&str; 4] = ["buyer", "supplier", "logistics", "admin"];

pub async fn register(State(state): State<AppState>, Json(body): Json<RegisterRequest>) -> AppResult<Json<AuthSession>> {
    if !VALID_ROLES.contains(&body.role.as_str()) {
        return Err(AppError::BadRequest(format!("Invalid role '{}'.", body.role)));
    }
    if body.password.len() < 6 {
        return Err(AppError::BadRequest("Password must be at least 6 characters.".to_string()));
    }

    let existing = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE lower(email) = lower($1)")
        .bind(&body.email)
        .fetch_one(&state.db)
        .await?;
    if existing > 0 {
        return Err(AppError::Conflict("An account with this email already exists.".to_string()));
    }

    let id = ids::user_id(&body.role);
    let password_hash = auth::hash_password(&body.password)?;
    let org_name = body.organization_name.clone().unwrap_or_else(|| body.name.clone());

    sqlx::query(
        r#"INSERT INTO users (id, email, password_hash, name, role, organization_name, phone, location, verified, profile_complete)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, TRUE)"#,
    )
    .bind(&id)
    .bind(&body.email)
    .bind(&password_hash)
    .bind(&body.name)
    .bind(&body.role)
    .bind(&org_name)
    .bind(&body.phone)
    .bind(&body.location)
    .execute(&state.db)
    .await?;

    services::log_audit(&state.db, AuditParams {
        action: "user_registered",
        actor_id: &id,
        actor_name: &body.name,
        actor_role: &body.role,
        entity_id: Some(&id),
        entity_type: Some("User"),
        detail: Some(&format!("{} registered as {}.", body.name, body.role)),
        transaction_id: None,
    }).await?;

    let token = auth::issue_token(&state.jwt_secret, &id, &body.role, &body.name, &body.email)?;
    Ok(Json(AuthSession { token, user_id: id, role: body.role, name: body.name, email: body.email }))
}

pub async fn login(State(state): State<AppState>, Json(body): Json<LoginRequest>) -> AppResult<Json<AuthSession>> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE lower(email) = lower($1)")
        .bind(&body.email)
        .fetch_optional(&state.db)
        .await?;

    let user = match user {
        Some(u) if auth::verify_password(&body.password, &u.password_hash) => u,
        _ => return Err(AppError::Unauthorized("Invalid email or password. Please verify your credentials.".to_string())),
    };

    let token = auth::issue_token(&state.jwt_secret, &user.id, &user.role, &user.name, &user.email)?;
    Ok(Json(AuthSession {
        token,
        user_id: user.id,
        role: user.role,
        name: user.name,
        email: user.email,
    }))
}

pub async fn me(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<User>> {
    let u = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(&user.id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found.".to_string()))?;
    Ok(Json(u))
}

pub async fn update_profile(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<UpdateProfileRequest>,
) -> AppResult<Json<User>> {
    let updated = sqlx::query_as::<_, User>(
        r#"UPDATE users SET
             name = COALESCE($1, name),
             organization_name = COALESCE($2, organization_name),
             phone = COALESCE($3, phone),
             location = COALESCE($4, location)
           WHERE id = $5
           RETURNING *"#,
    )
    .bind(&body.name)
    .bind(&body.organization_name)
    .bind(&body.phone)
    .bind(&body.location)
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("User not found.".to_string()))?;

    Ok(Json(updated))
}
