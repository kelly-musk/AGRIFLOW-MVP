use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    ids,
    models::{InitiateTransactionRequest, SupplyListing, Transaction, TransactionEvent, TransactionWithHistory, TransitionRequest},
    services::{self, AuditParams, NotifyParams},
    state::AppState,
};

async fn with_history(db: &sqlx::PgPool, txn: Transaction) -> AppResult<TransactionWithHistory> {
    let history = sqlx::query_as::<_, TransactionEvent>(
        "SELECT status, ts, actor, actor_role, note FROM transaction_events WHERE transaction_id = $1 ORDER BY ts ASC",
    )
    .bind(&txn.id)
    .fetch_all(db)
    .await?;
    Ok(TransactionWithHistory { transaction: txn, history })
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<InitiateTransactionRequest>,
) -> AppResult<Json<TransactionWithHistory>> {
    user.require_role("buyer")?;

    let listing = sqlx::query_as::<_, SupplyListing>("SELECT * FROM supply_listings WHERE id = $1")
        .bind(&body.listing_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Listing not found.".to_string()))?;

    if listing.status != "active" {
        return Err(AppError::Conflict("This listing is no longer active.".to_string()));
    }
    if body.quantity <= 0.0 {
        return Err(AppError::BadRequest("Quantity must be greater than zero.".to_string()));
    }

    let id = ids::transaction_id();
    let total_amount = body.quantity * listing.price_per_unit;

    let txn = sqlx::query_as::<_, Transaction>(
        r#"INSERT INTO transactions
             (id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name, commodity,
              quantity, unit, quality_grade, price_per_unit, total_amount, currency,
              pickup_location, delivery_location, expected_delivery_date, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, 'PENDING')
           RETURNING *"#,
    )
    .bind(&id)
    .bind(&listing.id)
    .bind(&body.demand_id)
    .bind(&user.id)
    .bind(&user.name)
    .bind(&listing.supplier_id)
    .bind(&listing.supplier_name)
    .bind(&listing.commodity)
    .bind(body.quantity)
    .bind(&listing.unit)
    .bind(&listing.quality_grade)
    .bind(listing.price_per_unit)
    .bind(total_amount)
    .bind(&listing.currency)
    .bind(&listing.location)
    .bind(&body.delivery_location)
    .bind(body.expected_delivery_date)
    .fetch_one(&state.db)
    .await?;

    sqlx::query(
        "INSERT INTO transaction_events (transaction_id, status, actor, actor_role, note) VALUES ($1, 'PENDING', $2, 'buyer', 'Transaction initiated by buyer.')",
    )
    .bind(&id)
    .bind(&user.name)
    .execute(&state.db)
    .await?;

    services::log_audit(&state.db, AuditParams {
        action: "transaction_initiated",
        actor_id: &user.id,
        actor_name: &user.name,
        actor_role: "buyer",
        entity_id: Some(&id),
        entity_type: Some("Transaction"),
        detail: Some(&format!(
            "Transaction {id} initiated for {} {} of {}. Value: ₦{}.",
            body.quantity, listing.unit, listing.commodity, total_amount
        )),
        transaction_id: Some(&id),
    }).await?;

    services::notify(&state.db, NotifyParams {
        user_id: &listing.supplier_id,
        kind: "transaction_request",
        title: "New Transaction Request",
        message: &format!(
            "{} has initiated a transaction for {} {} of {}. Reference: {id}",
            user.name, body.quantity, listing.unit, listing.commodity
        ),
        transaction_id: Some(&id),
    }).await?;

    Ok(Json(with_history(&state.db, txn).await?))
}

pub async fn list_mine(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<Vec<Transaction>>> {
    let rows = match user.role.as_str() {
        "buyer" => sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE buyer_id = $1 ORDER BY created_at DESC")
            .bind(&user.id)
            .fetch_all(&state.db)
            .await?,
        "supplier" => sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE supplier_id = $1 ORDER BY created_at DESC")
            .bind(&user.id)
            .fetch_all(&state.db)
            .await?,
        "admin" => sqlx::query_as::<_, Transaction>("SELECT * FROM transactions ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await?,
        _ => Vec::new(),
    };
    Ok(Json(rows))
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<TransactionWithHistory>> {
    let txn = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Transaction not found.".to_string()))?;

    let authorized = user.role == "admin"
        || txn.buyer_id == user.id
        || txn.supplier_id == user.id
        || (user.role == "logistics"
            && sqlx::query_scalar::<_, i64>("SELECT count(*) FROM logistics_jobs WHERE transaction_id = $1 AND provider_id = $2")
                .bind(&id)
                .bind(&user.id)
                .fetch_one(&state.db)
                .await? > 0);
    if !authorized {
        return Err(AppError::Forbidden("You do not have access to this transaction.".to_string()));
    }

    Ok(Json(with_history(&state.db, txn).await?))
}

pub async fn transition(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<TransitionRequest>,
) -> AppResult<Json<TransactionWithHistory>> {
    let updated = services::transition_transaction(
        &state.db,
        &id,
        &body.to,
        &user.id,
        &user.name,
        &user.role,
        body.note.as_deref(),
    )
    .await?;
    Ok(Json(with_history(&state.db, updated).await?))
}
