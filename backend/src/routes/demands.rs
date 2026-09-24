use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::ids;
use crate::json_extractor::AppJson;
use crate::models::demand::{CreateDemandRequest, DemandRequest};
use crate::models::user::UserRole;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct DemandQuery {
    pub commodity: Option<String>,
    pub status: Option<String>,
}

pub async fn list_open(
    State(state): State<AppState>,
    Query(q): Query<DemandQuery>,
) -> AppResult<Json<Vec<DemandRequest>>> {
    let status = q.status.unwrap_or_else(|| "open".to_string());
    let demands = sqlx::query_as!(
        DemandRequest,
        r#"
        SELECT id, buyer_id, buyer_name, commodity, quantity, unit, quality_grade,
               destination_location, required_by_date, indicative_budget, currency, notes,
               status as "status: _", created_at, updated_at
        FROM demand_requests
        WHERE status = $1 AND ($2::text IS NULL OR commodity = $2)
        ORDER BY created_at DESC
        "#,
        status,
        q.commodity,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(demands))
}

pub async fn mine(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<DemandRequest>>> {
    auth.require_role(UserRole::Buyer)?;

    let demands = sqlx::query_as!(
        DemandRequest,
        r#"
        SELECT id, buyer_id, buyer_name, commodity, quantity, unit, quality_grade,
               destination_location, required_by_date, indicative_budget, currency, notes,
               status as "status: _", created_at, updated_at
        FROM demand_requests
        WHERE buyer_id = $1
        ORDER BY created_at DESC
        "#,
        auth.user_id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(demands))
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<DemandRequest>> {
    let demand = sqlx::query_as!(
        DemandRequest,
        r#"
        SELECT id, buyer_id, buyer_name, commodity, quantity, unit, quality_grade,
               destination_location, required_by_date, indicative_budget, currency, notes,
               status as "status: _", created_at, updated_at
        FROM demand_requests WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Demand not found.".into()))?;

    Ok(Json(demand))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(body): AppJson<CreateDemandRequest>,
) -> AppResult<Json<DemandRequest>> {
    auth.require_role(UserRole::Buyer)?;

    if body.quantity <= rust_decimal::Decimal::ZERO {
        return Err(AppError::BadRequest("Quantity must be greater than zero.".into()));
    }

    let id = ids::generate("D");
    let currency = body.currency.unwrap_or_else(|| "NGN".to_string());

    let demand = sqlx::query_as!(
        DemandRequest,
        r#"
        INSERT INTO demand_requests
            (id, buyer_id, buyer_name, commodity, quantity, unit, quality_grade,
             destination_location, required_by_date, indicative_budget, currency, notes, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'open')
        RETURNING id, buyer_id, buyer_name, commodity, quantity, unit, quality_grade,
                  destination_location, required_by_date, indicative_budget, currency, notes,
                  status as "status: _", created_at, updated_at
        "#,
        id,
        auth.user_id,
        auth.name,
        body.commodity,
        body.quantity,
        body.unit,
        body.quality_grade,
        body.destination_location,
        body.required_by_date,
        body.indicative_budget,
        currency,
        body.notes,
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(demand))
}
