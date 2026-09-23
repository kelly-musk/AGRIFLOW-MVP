use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::json;

use crate::{auth::AuthUser, error::AppResult, models::Notification, state::AppState};

pub async fn mine(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<Notification>>> {
    let rows = sqlx::query_as::<_, Notification>("SELECT * FROM notifications WHERE user_id = $1 ORDER BY created_at DESC")
        .bind(&user.id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn unread_count(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<serde_json::Value>> {
    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notifications WHERE user_id = $1 AND read = FALSE")
        .bind(&user.id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(json!({ "count": count })))
}

pub async fn mark_read(State(state): State<AppState>, user: AuthUser, Path(id): Path<String>) -> AppResult<Json<serde_json::Value>> {
    sqlx::query("UPDATE notifications SET read = TRUE WHERE id = $1 AND user_id = $2")
        .bind(&id)
        .bind(&user.id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn mark_all_read(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<serde_json::Value>> {
    sqlx::query("UPDATE notifications SET read = TRUE WHERE user_id = $1")
        .bind(&user.id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}
