use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{AssignProviderRequest, LogisticsJob, RejectJobRequest, UpdateJobStatusRequest, User},
    services::{self, NotifyParams},
    state::AppState,
};

pub async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<LogisticsJob>> {
    let row = sqlx::query_as::<_, LogisticsJob>("SELECT * FROM logistics_jobs WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Logistics job not found.".to_string()))?;
    Ok(Json(row))
}

pub async fn for_transaction(State(state): State<AppState>, Path(transaction_id): Path<String>) -> AppResult<Json<Option<LogisticsJob>>> {
    let row = sqlx::query_as::<_, LogisticsJob>("SELECT * FROM logistics_jobs WHERE transaction_id = $1")
        .bind(&transaction_id)
        .fetch_optional(&state.db)
        .await?;
    Ok(Json(row))
}

pub async fn pending(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<LogisticsJob>>> {
    user.require_role("admin")?;
    let rows = sqlx::query_as::<_, LogisticsJob>("SELECT * FROM logistics_jobs WHERE status = 'PENDING' ORDER BY created_at ASC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn mine(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<LogisticsJob>>> {
    user.require_role("logistics")?;
    let rows = sqlx::query_as::<_, LogisticsJob>("SELECT * FROM logistics_jobs WHERE provider_id = $1 ORDER BY created_at DESC")
        .bind(&user.id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn providers(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<User>>> {
    user.require_role("admin")?;
    let rows = sqlx::query_as::<_, User>("SELECT * FROM users WHERE role = 'logistics' ORDER BY name ASC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn assign(
    State(state): State<AppState>,
    user: AuthUser,
    Path(job_id): Path<String>,
    Json(body): Json<AssignProviderRequest>,
) -> AppResult<Json<LogisticsJob>> {
    user.require_role("admin")?;

    let provider = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1 AND role = 'logistics'")
        .bind(&body.provider_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Logistics provider not found.".to_string()))?;

    let job = sqlx::query_as::<_, LogisticsJob>(
        "UPDATE logistics_jobs SET provider_id = $1, provider_name = $2, status = 'ASSIGNED', updated_at = now() WHERE id = $3 RETURNING *",
    )
    .bind(&provider.id)
    .bind(&provider.name)
    .bind(&job_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Logistics job not found.".to_string()))?;

    services::transition_transaction(
        &state.db,
        &job.transaction_id,
        "LOGISTICS_ASSIGNED",
        &user.id,
        &user.name,
        "admin",
        Some(&format!("{} assigned as logistics provider.", provider.name)),
    )
    .await?;

    services::notify(&state.db, NotifyParams {
        user_id: &provider.id,
        kind: "logistics_assigned",
        title: "New Logistics Assignment",
        message: &format!(
            "You have been assigned a logistics job: {} {} {} from {} to {}. Job: {}",
            job.commodity, job.quantity, job.unit, job.pickup_location, job.delivery_location, job.id
        ),
        transaction_id: Some(&job.transaction_id),
    }).await?;

    Ok(Json(job))
}

async fn update_status(
    state: &AppState,
    job_id: &str,
    provider_id: &str,
    provider_name: &str,
    status: &str,
    proof: Option<&UpdateJobStatusRequest>,
) -> AppResult<LogisticsJob> {
    let job = sqlx::query_as::<_, LogisticsJob>("SELECT * FROM logistics_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Logistics job not found.".to_string()))?;

    if job.provider_id.as_deref() != Some(provider_id) {
        return Err(AppError::Forbidden("Unauthorized: this job is not assigned to you.".to_string()));
    }

    let pod = proof.and_then(|p| p.proof_of_delivery.as_ref());
    let updated = sqlx::query_as::<_, LogisticsJob>(
        r#"UPDATE logistics_jobs SET
             status = $1,
             proof_recipient_name = COALESCE($2, proof_recipient_name),
             proof_delivery_note = COALESCE($3, proof_delivery_note),
             proof_timestamp = CASE WHEN $2 IS NOT NULL THEN now() ELSE proof_timestamp END,
             proof_recorded_by = COALESCE($4, proof_recorded_by),
             updated_at = now()
           WHERE id = $5
           RETURNING *"#,
    )
    .bind(status)
    .bind(pod.map(|p| p.recipient_name.as_str()))
    .bind(pod.map(|p| p.delivery_note.as_str()))
    .bind(pod.map(|_| provider_name))
    .bind(job_id)
    .fetch_one(&state.db)
    .await?;

    let txn_status = match status {
        "ACCEPTED" => Some("LOGISTICS_ACCEPTED"),
        "REJECTED" => Some("LOGISTICS_REJECTED"),
        "READY_FOR_PICKUP" => Some("READY_FOR_PICKUP"),
        "PICKED_UP" => Some("PICKED_UP"),
        "IN_TRANSIT" => Some("IN_TRANSIT"),
        "DELIVERED" => Some("DELIVERED"),
        _ => None,
    };

    if let Some(to) = txn_status {
        services::transition_transaction(
            &state.db,
            &job.transaction_id,
            to,
            provider_id,
            provider_name,
            "logistics",
            Some(&format!("Shipment status updated to {status}.")),
        )
        .await?;
    }

    Ok(updated)
}

pub async fn accept(State(state): State<AppState>, user: AuthUser, Path(job_id): Path<String>) -> AppResult<Json<LogisticsJob>> {
    user.require_role("logistics")?;
    let job = update_status(&state, &job_id, &user.id, &user.name, "ACCEPTED", None).await?;
    Ok(Json(job))
}

pub async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    Path(job_id): Path<String>,
    Json(_body): Json<RejectJobRequest>,
) -> AppResult<Json<LogisticsJob>> {
    user.require_role("logistics")?;
    let job = update_status(&state, &job_id, &user.id, &user.name, "REJECTED", None).await?;
    Ok(Json(job))
}

pub async fn set_status(
    State(state): State<AppState>,
    user: AuthUser,
    Path(job_id): Path<String>,
    Json(body): Json<UpdateJobStatusRequest>,
) -> AppResult<Json<LogisticsJob>> {
    user.require_role("logistics")?;
    let status = body.status.clone();
    let job = update_status(&state, &job_id, &user.id, &user.name, &status, Some(&body)).await?;
    Ok(Json(job))
}
