use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    ids,
    models::{FailPaymentRequest, InitiatePaymentRequest, Payment, Transaction},
    services::{self, AuditParams},
    state::AppState,
};

/// Simulated escrow: mirrors `paymentService.initiate` / `.confirm` / `.fail`.
/// No money actually moves — this exists so the transaction state machine and
/// audit trail behave exactly as a real payment processor integration would,
/// making it a drop-in point for Paystack/Flutterwave later.
pub async fn initiate(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<InitiatePaymentRequest>,
) -> AppResult<Json<Payment>> {
    user.require_role("buyer")?;

    if let Some(existing) = fetch_for_transaction(&state, &body.transaction_id).await? {
        if existing.status == "CONFIRMED" {
            return Err(AppError::Conflict("Payment has already been confirmed for this transaction.".to_string()));
        }
        if existing.status == "PENDING" {
            return Ok(Json(existing));
        }
    }

    let txn = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = $1")
        .bind(&body.transaction_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Transaction not found.".to_string()))?;
    if txn.buyer_id != user.id {
        return Err(AppError::Forbidden("Only the buyer may pay for this transaction.".to_string()));
    }

    services::transition_transaction(
        &state.db,
        &body.transaction_id,
        "PAYMENT_PENDING",
        &user.id,
        &user.name,
        "buyer",
        Some("Buyer initiated payment."),
    )
    .await?;

    let id = ids::payment_id(&body.transaction_id);
    let payment = sqlx::query_as::<_, Payment>(
        r#"INSERT INTO payments (id, transaction_id, payer_id, amount, currency, provider, status)
           VALUES ($1, $2, $3, $4, $5, 'AgriFlow Escrow Service', 'PENDING')
           RETURNING *"#,
    )
    .bind(&id)
    .bind(&body.transaction_id)
    .bind(&user.id)
    .bind(txn.total_amount)
    .bind(&txn.currency)
    .fetch_one(&state.db)
    .await?;

    services::log_audit(&state.db, AuditParams {
        action: "payment_initiated",
        actor_id: &user.id,
        actor_name: &user.name,
        actor_role: "buyer",
        entity_id: Some(&payment.id),
        entity_type: Some("Payment"),
        detail: Some(&format!("Payment {id} initiated for ₦{}.", txn.total_amount)),
        transaction_id: Some(&body.transaction_id),
    }).await?;

    sqlx::query("UPDATE transactions SET payment_id = $1 WHERE id = $2")
        .bind(&payment.id)
        .bind(&body.transaction_id)
        .execute(&state.db)
        .await?;

    Ok(Json(payment))
}

pub async fn confirm(State(state): State<AppState>, Path(payment_id): Path<String>) -> AppResult<Json<Payment>> {
    let payment = sqlx::query_as::<_, Payment>("SELECT * FROM payments WHERE id = $1")
        .bind(&payment_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Payment not found.".to_string()))?;

    if payment.status == "CONFIRMED" {
        return Ok(Json(payment));
    }

    let provider_ref = ids::provider_reference();
    let updated = sqlx::query_as::<_, Payment>(
        r#"UPDATE payments SET status = 'CONFIRMED', provider_reference = $1, updated_at = now(), completed_at = now()
           WHERE id = $2 RETURNING *"#,
    )
    .bind(&provider_ref)
    .bind(&payment_id)
    .fetch_one(&state.db)
    .await?;

    services::transition_transaction(
        &state.db,
        &payment.transaction_id,
        "PAYMENT_CONFIRMED",
        "system",
        "AgriFlow System",
        "system",
        Some(&format!("Payment confirmed. Provider ref: {provider_ref}")),
    )
    .await?;

    create_logistics_job(&state, &payment.transaction_id).await?;

    Ok(Json(updated))
}

pub async fn fail(
    State(state): State<AppState>,
    Path(payment_id): Path<String>,
    Json(body): Json<FailPaymentRequest>,
) -> AppResult<Json<Payment>> {
    let payment = sqlx::query_as::<_, Payment>("SELECT * FROM payments WHERE id = $1")
        .bind(&payment_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Payment not found.".to_string()))?;

    let updated = sqlx::query_as::<_, Payment>(
        "UPDATE payments SET status = 'FAILED', failure_reason = $1, updated_at = now() WHERE id = $2 RETURNING *",
    )
    .bind(&body.reason)
    .bind(&payment_id)
    .fetch_one(&state.db)
    .await?;

    services::transition_transaction(
        &state.db,
        &payment.transaction_id,
        "PAYMENT_FAILED",
        "system",
        "AgriFlow System",
        "system",
        Some(&format!("Payment failed: {}", body.reason)),
    )
    .await?;

    Ok(Json(updated))
}

pub async fn for_transaction(State(state): State<AppState>, Path(transaction_id): Path<String>) -> AppResult<Json<Option<Payment>>> {
    Ok(Json(fetch_for_transaction(&state, &transaction_id).await?))
}

async fn fetch_for_transaction(state: &AppState, transaction_id: &str) -> AppResult<Option<Payment>> {
    let row = sqlx::query_as::<_, Payment>("SELECT * FROM payments WHERE transaction_id = $1")
        .bind(transaction_id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row)
}

/// Auto-creates a logistics job once payment is confirmed — idempotent, same
/// as `logisticsService.createJobForTransaction` on the frontend.
async fn create_logistics_job(state: &AppState, transaction_id: &str) -> AppResult<()> {
    let existing = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM logistics_jobs WHERE transaction_id = $1")
        .bind(transaction_id)
        .fetch_one(&state.db)
        .await?;
    if existing > 0 {
        return Ok(());
    }

    let txn = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = $1")
        .bind(transaction_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Transaction not found.".to_string()))?;

    let job_id = ids::logistics_job_id();
    let logistics_cost = (txn.total_amount * 0.03).round();

    sqlx::query(
        r#"INSERT INTO logistics_jobs
             (id, transaction_id, commodity, quantity, unit, pickup_location, delivery_location,
              pickup_date, expected_delivery_date, logistics_cost, currency, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, now(), $8, $9, $10, 'PENDING')"#,
    )
    .bind(&job_id)
    .bind(transaction_id)
    .bind(&txn.commodity)
    .bind(txn.quantity)
    .bind(&txn.unit)
    .bind(&txn.pickup_location)
    .bind(&txn.delivery_location)
    .bind(txn.expected_delivery_date)
    .bind(logistics_cost)
    .bind(&txn.currency)
    .execute(&state.db)
    .await?;

    sqlx::query("UPDATE transactions SET logistics_job_id = $1 WHERE id = $2")
        .bind(&job_id)
        .bind(transaction_id)
        .execute(&state.db)
        .await?;

    services::transition_transaction(
        &state.db,
        transaction_id,
        "LOGISTICS_PENDING",
        "system",
        "AgriFlow System",
        "system",
        Some(&format!("Logistics job {job_id} created.")),
    )
    .await?;

    Ok(())
}
