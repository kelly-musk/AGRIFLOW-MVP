use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    ids,
    models::{Dispute, RaiseDisputeRequest, ResolveDisputeRequest, Transaction},
    services::{self, AuditParams, NotifyParams, PLATFORM_ADMIN_ID},
    state::AppState,
};

pub async fn raise(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<RaiseDisputeRequest>,
) -> AppResult<Json<Dispute>> {
    user.require_role("buyer")?;

    let txn = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = $1")
        .bind(&body.transaction_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Transaction not found.".to_string()))?;
    if txn.buyer_id != user.id {
        return Err(AppError::Forbidden("Only the buyer on this transaction may raise a dispute.".to_string()));
    }

    let id = ids::dispute_id();
    let dispute = sqlx::query_as::<_, Dispute>(
        r#"INSERT INTO disputes (id, transaction_id, raised_by_id, raised_by_name, reason, description, status)
           VALUES ($1, $2, $3, $4, $5, $6, 'OPEN')
           RETURNING *"#,
    )
    .bind(&id)
    .bind(&body.transaction_id)
    .bind(&user.id)
    .bind(&user.name)
    .bind(&body.reason)
    .bind(&body.description)
    .fetch_one(&state.db)
    .await?;

    services::transition_transaction(
        &state.db,
        &body.transaction_id,
        "DISPUTED",
        &user.id,
        &user.name,
        "buyer",
        Some(&format!("Dispute raised: {}", body.reason)),
    )
    .await?;

    sqlx::query("UPDATE transactions SET dispute_id = $1 WHERE id = $2")
        .bind(&id)
        .bind(&body.transaction_id)
        .execute(&state.db)
        .await?;

    services::log_audit(&state.db, AuditParams {
        action: "dispute_raised",
        actor_id: &user.id,
        actor_name: &user.name,
        actor_role: "buyer",
        entity_id: Some(&id),
        entity_type: Some("Dispute"),
        detail: Some(&format!("Dispute {id} raised. Reason: {}", body.reason)),
        transaction_id: Some(&body.transaction_id),
    }).await?;

    services::notify(&state.db, NotifyParams {
        user_id: PLATFORM_ADMIN_ID,
        kind: "dispute_raised",
        title: "Dispute Raised",
        message: &format!("A dispute has been raised on transaction {}. Reason: {}", body.transaction_id, body.reason),
        transaction_id: Some(&body.transaction_id),
    }).await?;

    Ok(Json(dispute))
}

pub async fn resolve(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ResolveDisputeRequest>,
) -> AppResult<Json<Dispute>> {
    user.require_role("admin")?;

    let dispute = sqlx::query_as::<_, Dispute>("SELECT * FROM disputes WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Dispute not found.".to_string()))?;

    let txn_status = match body.outcome.as_str() {
        "completed" => "COMPLETED",
        "cancelled" => "CANCELLED",
        _ => return Err(AppError::BadRequest("outcome must be 'completed' or 'cancelled'.".to_string())),
    };

    let updated = sqlx::query_as::<_, Dispute>(
        r#"UPDATE disputes SET status = 'RESOLVED', resolution = $1, resolved_by_id = $2, resolved_by_name = $3,
             resolved_at = now(), updated_at = now()
           WHERE id = $4
           RETURNING *"#,
    )
    .bind(&body.decision)
    .bind(&user.id)
    .bind(&user.name)
    .bind(&id)
    .fetch_one(&state.db)
    .await?;

    services::transition_transaction(
        &state.db,
        &dispute.transaction_id,
        txn_status,
        &user.id,
        &user.name,
        "admin",
        Some(&format!("Dispute resolved: {}", body.decision)),
    )
    .await?;

    services::log_audit(&state.db, AuditParams {
        action: "dispute_resolved",
        actor_id: &user.id,
        actor_name: &user.name,
        actor_role: "admin",
        entity_id: Some(&id),
        entity_type: Some("Dispute"),
        detail: Some(&format!("Dispute {id} resolved. Decision: {}. Outcome: {txn_status}.", body.decision)),
        transaction_id: Some(&dispute.transaction_id),
    }).await?;

    Ok(Json(updated))
}

pub async fn list_all(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<Dispute>>> {
    user.require_role("admin")?;
    let rows = sqlx::query_as::<_, Dispute>("SELECT * FROM disputes ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Dispute>> {
    let row = sqlx::query_as::<_, Dispute>("SELECT * FROM disputes WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Dispute not found.".to_string()))?;
    Ok(Json(row))
}
