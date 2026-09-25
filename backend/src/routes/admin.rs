//! RBAC-protected admin management endpoints (issues #33 and #35). Every
//! handler here requires `auth.require_role(UserRole::Admin)` as its first
//! line -- the same convention `require_role` already uses elsewhere (e.g.
//! `listings::create` gating on `Supplier`), rather than introducing a
//! separate middleware layer for just this module.
//!
//! `/api/admin/disputes*` and `/api/admin/audit` from issue #33's spec are
//! deliberately not implemented here: neither a `disputes` nor an
//! `audit_logs` table exists yet (tracked separately as #36 and #38).
//! Building stub endpoints with no real data behind them wouldn't be
//! meaningfully "done" -- they should land alongside those features.

use axum::{Json, extract::{Path, Query, State}};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::user::{User, UserPublic, UserRole};
use crate::state::AppState;

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    pub role: Option<String>,
    pub search: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUsersResponse {
    pub users: Vec<UserPublic>,
    pub total: i64,
    pub page: i64,
    pub limit: i64,
}

fn valid_role(role: &str) -> bool {
    matches!(role, "buyer" | "supplier" | "logistics" | "admin")
}

pub async fn list_users(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListUsersQuery>,
) -> AppResult<Json<ListUsersResponse>> {
    auth.require_role(UserRole::Admin)?;

    if let Some(role) = &q.role {
        if !valid_role(role) {
            return Err(AppError::BadRequest(format!("Unknown role: {role}")));
        }
    }

    let page = q.page.unwrap_or(1).max(1);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = (page - 1) * limit;

    let users = sqlx::query_as!(
        User,
        r#"
        SELECT id, email, password_hash, name, role as "role: _", organization_name, phone, location, verified, profile_complete, wallet_address, created_at, updated_at
        FROM users
        WHERE ($1::text IS NULL OR role = $1)
          AND ($2::text IS NULL
               OR name ILIKE '%' || $2 || '%'
               OR email ILIKE '%' || $2 || '%'
               OR COALESCE(organization_name, '') ILIKE '%' || $2 || '%')
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
        q.role,
        q.search,
        limit,
        offset,
    )
    .fetch_all(&state.db)
    .await?;

    let total = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) FROM users
        WHERE ($1::text IS NULL OR role = $1)
          AND ($2::text IS NULL
               OR name ILIKE '%' || $2 || '%'
               OR email ILIKE '%' || $2 || '%'
               OR COALESCE(organization_name, '') ILIKE '%' || $2 || '%')
        "#,
        q.role,
        q.search,
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0);

    Ok(Json(ListUsersResponse {
        users: users.into_iter().map(UserPublic::from).collect(),
        total,
        page,
        limit,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SetVerifiedRequest {
    pub verified: bool,
}

pub async fn set_verified(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<SetVerifiedRequest>,
) -> AppResult<Json<UserPublic>> {
    auth.require_role(UserRole::Admin)?;

    let user = sqlx::query_as!(
        User,
        r#"
        UPDATE users SET verified = $2, updated_at = now() WHERE id = $1
        RETURNING id, email, password_hash, name, role as "role: _", organization_name, phone, location, verified, profile_complete, wallet_address, created_at, updated_at
        "#,
        id,
        body.verified,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("User not found.".into()))?;

    Ok(Json(UserPublic::from(user)))
}
