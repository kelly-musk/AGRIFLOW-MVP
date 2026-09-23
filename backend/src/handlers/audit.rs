use axum::{
    Json,
    extract::{Path, State},
};

use crate::{auth::AuthUser, error::AppResult, models::AuditEvent, state::AppState};

pub async fn list_all(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<AuditEvent>>> {
    user.require_role("admin")?;
    let rows = sqlx::query_as::<_, AuditEvent>("SELECT * FROM audit_events ORDER BY created_at DESC LIMIT 500")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn for_transaction(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(transaction_id): Path<String>,
) -> AppResult<Json<Vec<AuditEvent>>> {
    let rows = sqlx::query_as::<_, AuditEvent>("SELECT * FROM audit_events WHERE transaction_id = $1 ORDER BY created_at ASC")
        .bind(&transaction_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}
