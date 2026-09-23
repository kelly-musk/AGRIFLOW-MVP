use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    ids,
    models::{CreateSupplyRequest, SupplyListing, UpdateSupplyRequest},
    services::{self, AuditParams},
    state::AppState,
};

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateSupplyRequest>,
) -> AppResult<Json<SupplyListing>> {
    user.require_role("supplier")?;

    let id = ids::supply_id();
    let listing = sqlx::query_as::<_, SupplyListing>(
        r#"INSERT INTO supply_listings
             (id, supplier_id, supplier_name, supplier_verified, commodity, quantity, unit,
              quality_grade, price_per_unit, currency, location, availability_date, description, status)
           VALUES ($1, $2, $3, TRUE, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active')
           RETURNING *"#,
    )
    .bind(&id)
    .bind(&user.id)
    .bind(&user.name)
    .bind(&body.commodity)
    .bind(body.quantity)
    .bind(&body.unit)
    .bind(&body.quality_grade)
    .bind(body.price_per_unit)
    .bind(&body.currency)
    .bind(&body.location)
    .bind(body.availability_date)
    .bind(&body.description)
    .fetch_one(&state.db)
    .await?;

    services::log_audit(&state.db, AuditParams {
        action: "supply_created",
        actor_id: &user.id,
        actor_name: &user.name,
        actor_role: "supplier",
        entity_id: Some(&listing.id),
        entity_type: Some("SupplyListing"),
        detail: Some(&format!("{} {} of {} listed at ₦{}/{}.", body.quantity, body.unit, body.commodity, body.price_per_unit, body.unit)),
        transaction_id: None,
    }).await?;

    Ok(Json(listing))
}

pub async fn list_active(State(state): State<AppState>) -> AppResult<Json<Vec<SupplyListing>>> {
    let rows = sqlx::query_as::<_, SupplyListing>("SELECT * FROM supply_listings WHERE status = 'active' ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<SupplyListing>> {
    let row = sqlx::query_as::<_, SupplyListing>("SELECT * FROM supply_listings WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Listing not found.".to_string()))?;
    Ok(Json(row))
}

pub async fn mine(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<SupplyListing>>> {
    user.require_role("supplier")?;
    let rows = sqlx::query_as::<_, SupplyListing>("SELECT * FROM supply_listings WHERE supplier_id = $1 ORDER BY created_at DESC")
        .bind(&user.id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateSupplyRequest>,
) -> AppResult<Json<SupplyListing>> {
    user.require_role("supplier")?;

    let existing = sqlx::query_as::<_, SupplyListing>("SELECT * FROM supply_listings WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Listing not found.".to_string()))?;
    if existing.supplier_id != user.id {
        return Err(AppError::Forbidden("Unauthorized: you do not own this listing.".to_string()));
    }

    let updated = sqlx::query_as::<_, SupplyListing>(
        r#"UPDATE supply_listings SET
             quantity = COALESCE($1, quantity),
             price_per_unit = COALESCE($2, price_per_unit),
             description = COALESCE($3, description),
             status = COALESCE($4, status),
             updated_at = now()
           WHERE id = $5
           RETURNING *"#,
    )
    .bind(body.quantity)
    .bind(body.price_per_unit)
    .bind(&body.description)
    .bind(&body.status)
    .bind(&id)
    .fetch_one(&state.db)
    .await?;

    services::log_audit(&state.db, AuditParams {
        action: "supply_updated",
        actor_id: &user.id,
        actor_name: &user.name,
        actor_role: "supplier",
        entity_id: Some(&id),
        entity_type: Some("SupplyListing"),
        detail: Some(&format!("Listing {id} updated.")),
        transaction_id: None,
    }).await?;

    Ok(Json(updated))
}
