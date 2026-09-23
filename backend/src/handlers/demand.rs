use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    ids,
    models::{CreateDemandRequest, DemandRequest},
    services::{self, AuditParams},
    state::AppState,
};

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateDemandRequest>,
) -> AppResult<Json<DemandRequest>> {
    user.require_role("buyer")?;

    let id = ids::demand_id();
    let demand = sqlx::query_as::<_, DemandRequest>(
        r#"INSERT INTO demand_requests
             (id, buyer_id, buyer_name, commodity, quantity, unit, quality_grade,
              destination_location, required_by_date, indicative_budget, currency, notes, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'open')
           RETURNING *"#,
    )
    .bind(&id)
    .bind(&user.id)
    .bind(&user.name)
    .bind(&body.commodity)
    .bind(body.quantity)
    .bind(&body.unit)
    .bind(&body.quality_grade)
    .bind(&body.destination_location)
    .bind(body.required_by_date)
    .bind(body.indicative_budget)
    .bind(&body.currency)
    .bind(&body.notes)
    .fetch_one(&state.db)
    .await?;

    services::log_audit(&state.db, AuditParams {
        action: "demand_created",
        actor_id: &user.id,
        actor_name: &user.name,
        actor_role: "buyer",
        entity_id: Some(&demand.id),
        entity_type: Some("DemandRequest"),
        detail: Some(&format!(
            "Demand for {} {} of {}, delivery to {}.",
            body.quantity, body.unit, body.commodity, body.destination_location
        )),
        transaction_id: None,
    }).await?;

    Ok(Json(demand))
}

pub async fn list_all(State(state): State<AppState>) -> AppResult<Json<Vec<DemandRequest>>> {
    let rows = sqlx::query_as::<_, DemandRequest>("SELECT * FROM demand_requests ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<DemandRequest>> {
    let row = sqlx::query_as::<_, DemandRequest>("SELECT * FROM demand_requests WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Demand not found.".to_string()))?;
    Ok(Json(row))
}

pub async fn mine(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<DemandRequest>>> {
    user.require_role("buyer")?;
    let rows = sqlx::query_as::<_, DemandRequest>("SELECT * FROM demand_requests WHERE buyer_id = $1 ORDER BY created_at DESC")
        .bind(&user.id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}
