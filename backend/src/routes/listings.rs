use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::ids;
use crate::models::listing::{CreateListingRequest, ListingStatus, SupplyListing, UpdateListingRequest};
use crate::models::user::UserRole;
use crate::state::AppState;
use crate::validation::require_non_empty;

#[derive(Debug, Deserialize)]
pub struct ListingQuery {
    pub commodity: Option<String>,
    pub status: Option<String>,
}

/// Public: browsing what's for sale shouldn't require an account. This
/// endpoint returns only marketplace-facing fields (see `SupplyListing`) --
/// no email, phone, or other contact info -- so there's nothing gated by
/// requiring a JWT here beyond making sign-up friction for lookers.
pub async fn list_active(
    State(state): State<AppState>,
    Query(q): Query<ListingQuery>,
) -> AppResult<Json<Vec<SupplyListing>>> {
    let status = q.status.unwrap_or_else(|| "active".to_string());
    let listings = sqlx::query_as!(
        SupplyListing,
        r#"
        SELECT id, supplier_id, supplier_name, supplier_verified, commodity, quantity,
               unit, quality_grade, price_per_unit, currency, location, availability_date,
               description, status as "status: _", created_at, updated_at
        FROM supply_listings
        WHERE status = $1 AND ($2::text IS NULL OR commodity = $2)
        ORDER BY created_at DESC
        "#,
        status,
        q.commodity,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(listings))
}

pub async fn mine(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<SupplyListing>>> {
    auth.require_role(UserRole::Supplier)?;

    let listings = sqlx::query_as!(
        SupplyListing,
        r#"
        SELECT id, supplier_id, supplier_name, supplier_verified, commodity, quantity,
               unit, quality_grade, price_per_unit, currency, location, availability_date,
               description, status as "status: _", created_at, updated_at
        FROM supply_listings
        WHERE supplier_id = $1
        ORDER BY created_at DESC
        "#,
        auth.user_id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(listings))
}

/// Public, same reasoning as `list_active`.
pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<SupplyListing>> {
    let listing = sqlx::query_as!(
        SupplyListing,
        r#"
        SELECT id, supplier_id, supplier_name, supplier_verified, commodity, quantity,
               unit, quality_grade, price_per_unit, currency, location, availability_date,
               description, status as "status: _", created_at, updated_at
        FROM supply_listings WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Listing not found.".into()))?;

    Ok(Json(listing))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateListingRequest>,
) -> AppResult<Json<SupplyListing>> {
    auth.require_role(UserRole::Supplier)?;

    require_non_empty("commodity", &body.commodity)?;
    require_non_empty("unit", &body.unit)?;
    require_non_empty("qualityGrade", &body.quality_grade)?;
    require_non_empty("location", &body.location)?;

    if body.quantity <= rust_decimal::Decimal::ZERO {
        return Err(AppError::BadRequest("Quantity must be greater than zero.".into()));
    }
    if body.price_per_unit < rust_decimal::Decimal::ZERO {
        return Err(AppError::BadRequest("Price cannot be negative.".into()));
    }

    let currency = body.currency.unwrap_or_else(|| "NGN".to_string());
    let description = body.description.unwrap_or_default();

    let mut listing = None;
    for _ in 0..ids::MAX_ID_ATTEMPTS {
        let id = ids::generate("SUP");
        match sqlx::query_as!(
            SupplyListing,
            r#"
            INSERT INTO supply_listings
                (id, supplier_id, supplier_name, supplier_verified, commodity, quantity, unit,
                 quality_grade, price_per_unit, currency, location, availability_date, description, status)
            VALUES ($1, $2, $3, TRUE, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active')
            RETURNING id, supplier_id, supplier_name, supplier_verified, commodity, quantity,
                      unit, quality_grade, price_per_unit, currency, location, availability_date,
                      description, status as "status: _", created_at, updated_at
            "#,
            id,
            auth.user_id,
            auth.name,
            body.commodity,
            body.quantity,
            body.unit,
            body.quality_grade,
            body.price_per_unit,
            currency,
            body.location,
            body.availability_date,
            description,
        )
        .fetch_one(&state.db)
        .await
        {
            Ok(l) => {
                listing = Some(l);
                break;
            }
            Err(e) if ids::is_id_collision(&e) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    let listing = listing.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "failed to generate a unique listing id after {} attempts",
            ids::MAX_ID_ATTEMPTS
        ))
    })?;

    Ok(Json(listing))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateListingRequest>,
) -> AppResult<Json<SupplyListing>> {
    auth.require_role(UserRole::Supplier)?;

    let existing = sqlx::query_scalar!("SELECT supplier_id FROM supply_listings WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Listing not found.".into()))?;

    if existing != auth.user_id {
        return Err(AppError::Forbidden("You do not own this listing.".into()));
    }

    let status: Option<ListingStatus> = body.status;

    let listing = sqlx::query_as!(
        SupplyListing,
        r#"
        UPDATE supply_listings SET
            quantity = COALESCE($2, quantity),
            price_per_unit = COALESCE($3, price_per_unit),
            description = COALESCE($4, description),
            status = COALESCE($5, status),
            updated_at = now()
        WHERE id = $1
        RETURNING id, supplier_id, supplier_name, supplier_verified, commodity, quantity,
                  unit, quality_grade, price_per_unit, currency, location, availability_date,
                  description, status as "status: _", created_at, updated_at
        "#,
        id,
        body.quantity,
        body.price_per_unit,
        body.description,
        status as _,
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(listing))
}
