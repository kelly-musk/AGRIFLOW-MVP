use axum::{
    Json,
    extract::{Path, State},
};
use std::str::FromStr;

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::ids;
use crate::models::listing::SupplyListing;
use crate::models::payment::{
    ConfirmPaymentRequest, CreateBachsSessionRequest, CreateBachsSessionResponse,
    InitiatePaymentRequest, Payment,
};
use crate::models::transaction::{
    CreateTransactionRequest, MockPaymentFailRequest, Transaction, TransactionEvent,
    TransactionWithHistory, TransitionRequest,
};
use crate::models::user::UserRole;
use crate::state::AppState;
use crate::state_machine::{Actor, TransactionStatus, can_actor_transition};

fn role_to_actor(role: UserRole) -> Actor {
    match role {
        UserRole::Buyer => Actor::Buyer,
        UserRole::Supplier => Actor::Supplier,
        UserRole::Logistics => Actor::Logistics,
        UserRole::Admin => Actor::Admin,
    }
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateTransactionRequest>,
) -> AppResult<Json<TransactionWithHistory>> {
    auth.require_role(UserRole::Buyer)?;

    if body.quantity <= rust_decimal::Decimal::ZERO {
        return Err(AppError::BadRequest("Quantity must be greater than zero.".into()));
    }

    let listing = sqlx::query_as!(
        SupplyListing,
        r#"
        SELECT id, supplier_id, supplier_name, supplier_verified, commodity, quantity,
               unit, quality_grade, price_per_unit, currency, location, availability_date,
               description, status as "status: _", created_at, updated_at
        FROM supply_listings WHERE id = $1
        "#,
        body.listing_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Listing not found.".into()))?;

    if listing.supplier_id == auth.user_id {
        return Err(AppError::BadRequest("You cannot buy from your own listing.".into()));
    }
    if body.quantity > listing.quantity {
        return Err(AppError::BadRequest(format!(
            "Only {} {} available in this listing.",
            listing.quantity, listing.unit
        )));
    }

    let id = ids::generate("TXN-AGF");
    let total_amount = body.quantity * listing.price_per_unit;

    let mut tx = state.db.begin().await?;

    let txn = sqlx::query_as!(
        Transaction,
        r#"
        INSERT INTO transactions
            (id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
             commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
             pickup_location, delivery_location, expected_delivery_date, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, 'PENDING')
        RETURNING id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                  commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                  pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                  logistics_job_id, dispute_id, created_at, updated_at
        "#,
        id,
        listing.id,
        body.demand_id,
        auth.user_id,
        auth.name,
        listing.supplier_id,
        listing.supplier_name,
        listing.commodity,
        body.quantity,
        listing.unit,
        listing.quality_grade,
        listing.price_per_unit,
        total_amount,
        listing.currency,
        listing.location,
        body.delivery_location,
        body.expected_delivery_date,
    )
    .fetch_one(&mut *tx)
    .await?;

    let event = sqlx::query_as!(
        TransactionEvent,
        r#"
        INSERT INTO transaction_events (transaction_id, status, actor, actor_role, note)
        VALUES ($1, 'PENDING', $2, 'buyer', 'Transaction initiated by buyer.')
        RETURNING id, transaction_id, status, actor, actor_role, note, created_at
        "#,
        txn.id,
        auth.name,
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(TransactionWithHistory {
        transaction: txn,
        history: vec![event],
    }))
}

pub async fn list_mine(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<Transaction>>> {
    let txns = match auth.role {
        UserRole::Buyer => {
            sqlx::query_as!(
                Transaction,
                r#"SELECT id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                          commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                          pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                          logistics_job_id, dispute_id, created_at, updated_at
                   FROM transactions WHERE buyer_id = $1 ORDER BY created_at DESC"#,
                auth.user_id,
            )
            .fetch_all(&state.db)
            .await?
        }
        UserRole::Supplier => {
            sqlx::query_as!(
                Transaction,
                r#"SELECT id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                          commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                          pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                          logistics_job_id, dispute_id, created_at, updated_at
                   FROM transactions WHERE supplier_id = $1 ORDER BY created_at DESC"#,
                auth.user_id,
            )
            .fetch_all(&state.db)
            .await?
        }
        UserRole::Admin => {
            sqlx::query_as!(
                Transaction,
                r#"SELECT id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                          commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                          pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                          logistics_job_id, dispute_id, created_at, updated_at
                   FROM transactions ORDER BY created_at DESC"#,
            )
            .fetch_all(&state.db)
            .await?
        }
        UserRole::Logistics => Vec::new(),
    };

    Ok(Json(txns))
}

async fn load_transaction(state: &AppState, id: &str) -> AppResult<Transaction> {
    sqlx::query_as!(
        Transaction,
        r#"SELECT id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                  commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                  pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                  logistics_job_id, dispute_id, created_at, updated_at
           FROM transactions WHERE id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Transaction not found.".into()))
}

fn assert_participant_or_admin(auth: &AuthUser, txn: &Transaction) -> AppResult<()> {
    let is_participant = auth.user_id == txn.buyer_id || auth.user_id == txn.supplier_id;
    if auth.role == UserRole::Admin || is_participant {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "You are not a participant in this transaction.".into(),
        ))
    }
}

pub async fn get_one(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<TransactionWithHistory>> {
    let txn = load_transaction(&state, &id).await?;
    assert_participant_or_admin(&auth, &txn)?;

    let history = sqlx::query_as!(
        TransactionEvent,
        r#"SELECT id, transaction_id, status, actor, actor_role, note, created_at
           FROM transaction_events WHERE transaction_id = $1 ORDER BY created_at ASC"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(TransactionWithHistory {
        transaction: txn,
        history,
    }))
}

pub async fn transition(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<TransitionRequest>,
) -> AppResult<Json<TransactionWithHistory>> {
    let txn = load_transaction(&state, &id).await?;

    // Buyers and suppliers may only drive transactions they're actually part
    // of; admin/system/logistics are broader roles until jobs/assignment
    // tables exist, so they're checked purely via the state machine's actor
    // table for now.
    if matches!(auth.role, UserRole::Buyer | UserRole::Supplier) {
        assert_participant_or_admin(&auth, &txn)?;
    }

    let from = TransactionStatus::from_str(&txn.status)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    let to = TransactionStatus::from_str(&body.to).map_err(AppError::BadRequest)?;
    let actor = role_to_actor(auth.role);

    let check = can_actor_transition(from, to, actor);
    if !check.allowed {
        return Err(AppError::Conflict(
            check.reason.unwrap_or_else(|| "Transition not permitted.".into()),
        ));
    }

    let mut db_tx = state.db.begin().await?;

    let updated = sqlx::query_as!(
        Transaction,
        r#"
        UPDATE transactions SET status = $2, updated_at = now() WHERE id = $1
        RETURNING id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                  commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                  pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                  logistics_job_id, dispute_id, created_at, updated_at
        "#,
        id,
        to.as_str(),
    )
    .fetch_one(&mut *db_tx)
    .await?;

    sqlx::query!(
        r#"INSERT INTO transaction_events (transaction_id, status, actor, actor_role, note)
           VALUES ($1, $2, $3, $4, $5)"#,
        id,
        to.as_str(),
        auth.name,
        actor.to_string(),
        body.note,
    )
    .execute(&mut *db_tx)
    .await?;

    db_tx.commit().await?;

    let history = sqlx::query_as!(
        TransactionEvent,
        r#"SELECT id, transaction_id, status, actor, actor_role, note, created_at
           FROM transaction_events WHERE transaction_id = $1 ORDER BY created_at ASC"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(TransactionWithHistory {
        transaction: updated,
        history,
    }))
}

fn assert_is_buyer_on_txn(auth: &AuthUser, txn: &Transaction) -> AppResult<()> {
    if auth.role != UserRole::Buyer || auth.user_id != txn.buyer_id {
        return Err(AppError::Forbidden(
            "Only the buyer on this transaction can settle its payment.".into(),
        ));
    }
    Ok(())
}

/// Applies one system-actor transition inside an already-open db transaction,
/// checking it against the state machine the same way the generic
/// `transition` handler does, and records the event. Used by the mock
/// payment endpoints below, which are the only place a request is allowed
/// to act as `Actor::System` — see their doc comments for why.
async fn apply_system_transition(
    db_tx: &mut sqlx::PgConnection,
    id: &str,
    to: TransactionStatus,
    note: &str,
) -> AppResult<Transaction> {
    let current = sqlx::query_as!(
        Transaction,
        r#"SELECT id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                  commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                  pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                  logistics_job_id, dispute_id, created_at, updated_at
           FROM transactions WHERE id = $1 FOR UPDATE"#,
        id,
    )
    .fetch_one(&mut *db_tx)
    .await?;

    let from = TransactionStatus::from_str(&current.status)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    let check = can_actor_transition(from, to, Actor::System);
    if !check.allowed {
        return Err(AppError::Conflict(
            check.reason.unwrap_or_else(|| "Transition not permitted.".into()),
        ));
    }

    let updated = sqlx::query_as!(
        Transaction,
        r#"
        UPDATE transactions SET status = $2, updated_at = now() WHERE id = $1
        RETURNING id, listing_id, demand_id, buyer_id, buyer_name, supplier_id, supplier_name,
                  commodity, quantity, unit, quality_grade, price_per_unit, total_amount, currency,
                  pickup_location, delivery_location, expected_delivery_date, status, payment_id,
                  logistics_job_id, dispute_id, created_at, updated_at
        "#,
        id,
        to.as_str(),
    )
    .fetch_one(&mut *db_tx)
    .await?;

    sqlx::query!(
        r#"INSERT INTO transaction_events (transaction_id, status, actor, actor_role, note)
           VALUES ($1, $2, 'AgriFlow System', 'system', $3)"#,
        id,
        to.as_str(),
        note,
    )
    .execute(&mut *db_tx)
    .await?;

    Ok(updated)
}

async fn history_for(state: &AppState, id: &str) -> AppResult<Vec<TransactionEvent>> {
    Ok(sqlx::query_as!(
        TransactionEvent,
        r#"SELECT id, transaction_id, status, actor, actor_role, note, created_at
           FROM transaction_events WHERE transaction_id = $1 ORDER BY created_at ASC"#,
        id,
    )
    .fetch_all(&state.db)
    .await?)
}

async fn payment_for_txn(state: &AppState, transaction_id: &str) -> AppResult<Option<Payment>> {
    Ok(sqlx::query_as!(
        Payment,
        r#"SELECT id, transaction_id, payer_id, amount, currency, provider, provider_reference,
                  stellar_tx_hash, bachs_session_id, status, failure_reason, created_at, updated_at, completed_at
           FROM payments WHERE transaction_id = $1"#,
        transaction_id,
    )
    .fetch_optional(&state.db)
    .await?)
}

async fn payment_by_bachs_session(state: &AppState, checkout_id: &str) -> AppResult<Option<Payment>> {
    Ok(sqlx::query_as!(
        Payment,
        r#"SELECT id, transaction_id, payer_id, amount, currency, provider, provider_reference,
                  stellar_tx_hash, bachs_session_id, status, failure_reason, created_at, updated_at, completed_at
           FROM payments WHERE bachs_session_id = $1"#,
        checkout_id,
    )
    .fetch_optional(&state.db)
    .await?)
}

/// Shared settlement core: marks a payment CONFIRMED (if not already) and
/// drives the transaction PaymentConfirmed -> LogisticsPending. Idempotent.
/// Used by both `mock_confirm_payment` (buyer-facing, only for rails with
/// no real provider) and the Bachs webhook handler (the only legitimate
/// confirmation path once a payment's provider is Bachs).
async fn settle_payment(
    state: &AppState,
    transaction_id: &str,
    payment_id: &str,
    provider_reference: Option<String>,
    stellar_tx_hash: Option<String>,
) -> AppResult<TransactionWithHistory> {
    let txn = load_transaction(state, transaction_id).await?;

    let current_status: String =
        sqlx::query_scalar!("SELECT status FROM payments WHERE id = $1", payment_id)
            .fetch_one(&state.db)
            .await?;

    if current_status != "CONFIRMED" {
        let reference = provider_reference.unwrap_or_else(ids::provider_reference);
        sqlx::query!(
            r#"UPDATE payments SET
                 status = 'CONFIRMED',
                 provider_reference = COALESCE(provider_reference, $2),
                 stellar_tx_hash = COALESCE($3, stellar_tx_hash),
                 completed_at = COALESCE(completed_at, now()),
                 updated_at = now()
               WHERE id = $1"#,
            payment_id,
            reference,
            stellar_tx_hash,
        )
        .execute(&state.db)
        .await?;
    }

    // Idempotent: a retry (network hiccup, double-click, a re-delivered
    // webhook) after the transaction already reached LOGISTICS_PENDING must
    // not fail -- the transitions below aren't valid to run twice (Postgres
    // would reject LOGISTICS_PENDING -> PAYMENT_CONFIRMED as illegal), so
    // just return the already-settled state.
    if txn.status == "LOGISTICS_PENDING" {
        let history = history_for(state, transaction_id).await?;
        return Ok(TransactionWithHistory { transaction: txn, history });
    }

    let mut db_tx = state.db.begin().await?;
    apply_system_transition(
        &mut db_tx,
        transaction_id,
        TransactionStatus::PaymentConfirmed,
        "Payment confirmed.",
    )
    .await?;
    let transaction = apply_system_transition(
        &mut db_tx,
        transaction_id,
        TransactionStatus::LogisticsPending,
        "Logistics job queued.",
    )
    .await?;
    db_tx.commit().await?;

    let history = history_for(state, transaction_id).await?;
    Ok(TransactionWithHistory { transaction, history })
}

/// Creates the pending payment record for a transaction the buyer is about
/// to pay -- the durable counterpart to the ACCEPTED -> PAYMENT_PENDING
/// transition the generic /transition endpoint already allows a buyer to
/// drive. Idempotent: calling it again for the same transaction returns the
/// existing record rather than creating a second one.
pub async fn initiate_payment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<InitiatePaymentRequest>,
) -> AppResult<Json<Payment>> {
    let txn = load_transaction(&state, &id).await?;
    assert_is_buyer_on_txn(&auth, &txn)?;

    if let Some(existing) = payment_for_txn(&state, &id).await? {
        return Ok(Json(existing));
    }

    if body.amount <= rust_decimal::Decimal::ZERO {
        return Err(AppError::BadRequest("Amount must be greater than zero.".into()));
    }

    // NOTE: single-attempt id generation, matching every other insert on
    // this branch of main today. The collision-retry helper (ids::generate
    // with a retry loop) lives in a separate, not-yet-merged PR
    // (fix/id-collision-retry) -- once that lands, this insert should be
    // updated to use it the same way listings/demands/users/transactions
    // do, rather than duplicating that mechanism here first.
    let payment_id = ids::generate("PAY-AGF");
    let payment = sqlx::query_as!(
        Payment,
        r#"
        INSERT INTO payments (id, transaction_id, payer_id, amount, currency, status)
        VALUES ($1, $2, $3, $4, $5, 'PENDING')
        RETURNING id, transaction_id, payer_id, amount, currency, provider, provider_reference,
                  stellar_tx_hash, bachs_session_id, status, failure_reason, created_at, updated_at, completed_at
        "#,
        payment_id,
        id,
        auth.user_id,
        body.amount,
        body.currency,
    )
    .fetch_one(&state.db)
    .await?;

    sqlx::query!("UPDATE transactions SET payment_id = $1 WHERE id = $2", payment.id, id)
        .execute(&state.db)
        .await?;

    Ok(Json(payment))
}

/// Settles the mock escrow payment for a transaction the buyer initiated,
/// then immediately queues it for logistics. Only for rails with no real
/// provider verification yet -- **not** for Bachs-sourced payments, which
/// can only be confirmed by `bachs_webhook` once Bachs itself confirms the
/// money moved (see that handler's doc comment for why letting the buyer
/// confirm their own Bachs payment would be a real, provable exploit).
///
/// The amount/currency being settled are never taken from the request body
/// here -- only from the payment row `initiate_payment` already created.
/// Letting the caller redeclare the amount at confirm time would let a
/// buyer "confirm" a payment for less than they actually owed.
pub async fn mock_confirm_payment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ConfirmPaymentRequest>,
) -> AppResult<Json<TransactionWithHistory>> {
    let txn = load_transaction(&state, &id).await?;
    assert_is_buyer_on_txn(&auth, &txn)?;

    let payment = payment_for_txn(&state, &id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Call payment/initiate before confirming.".into()))?;

    if payment.provider == "Bachs" {
        return Err(AppError::Conflict(
            "Bachs payments are confirmed automatically once payment completes -- they can't be confirmed directly.".into(),
        ));
    }

    let result = settle_payment(&state, &id, &payment.id, None, body.stellar_tx_hash).await?;
    Ok(Json(result))
}

/// Creates a Bachs.io hosted checkout session server-side. The secret key
/// never reaches the browser -- the frontend gets back only the checkout
/// URL to redirect to. Marks the payment's provider as `Bachs`, which is
/// what makes `mock_confirm_payment` refuse to confirm it directly.
pub async fn create_bachs_checkout_session(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<CreateBachsSessionRequest>,
) -> AppResult<Json<CreateBachsSessionResponse>> {
    let txn = load_transaction(&state, &id).await?;
    assert_is_buyer_on_txn(&auth, &txn)?;

    let payment = payment_for_txn(&state, &id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Call payment/initiate before starting checkout.".into()))?;

    if payment.status == "CONFIRMED" {
        return Err(AppError::Conflict("Payment already confirmed.".into()));
    }

    let user = sqlx::query!("SELECT email, name FROM users WHERE id = $1", auth.user_id)
        .fetch_one(&state.db)
        .await?;

    let success_url = body.success_url.unwrap_or_else(|| {
        format!("https://agri-flowmvp.vercel.app/app/transactions/{id}?payment=success")
    });
    let cancel_url = body.cancel_url.unwrap_or_else(|| {
        format!("https://agri-flowmvp.vercel.app/app/transactions/{id}/pay?payment=cancelled")
    });

    let session = state
        .bachs
        .create_checkout_session(
            payment.amount,
            &payment.currency,
            &user.email,
            &user.name,
            success_url,
            cancel_url,
            &id,
        )
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Bachs checkout session creation failed: {e}")))?;

    sqlx::query!(
        "UPDATE payments SET provider = 'Bachs', bachs_session_id = $2, updated_at = now() WHERE id = $1",
        payment.id,
        session.checkout_id,
    )
    .execute(&state.db)
    .await?;

    Ok(Json(CreateBachsSessionResponse { checkout_url: session.checkout_url }))
}

/// Bachs webhook receiver -- public, no JWT (Bachs has no user session to
/// present). This is the **only** legitimate way a Bachs-sourced payment
/// gets confirmed: `mock_confirm_payment` explicitly refuses payments with
/// `provider = 'Bachs'`, because trusting the buyer's own request (or the
/// checkout redirect's `?payment=success` query param, which is just as
/// forgeable) would let a buyer mark their own payment confirmed without
/// ever paying -- exactly the class of exploit already closed once for the
/// generic transition endpoint. See API_AUDIT.md.
///
/// Verifies the HMAC-SHA256 signature over the raw body before parsing
/// anything (see bachs.rs), and always returns 200 once a delivery is
/// understood (even for an unknown checkout_id, or an event type this app
/// doesn't act on) so Bachs doesn't retry forever -- only a bad signature
/// is rejected.
pub async fn bachs_webhook(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> AppResult<axum::http::StatusCode> {
    let signature = headers
        .get("X-Bachs-Signature-V2")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let timestamp = headers
        .get("X-Bachs-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !state.bachs.verify_webhook(&body, signature, timestamp) {
        return Err(AppError::Unauthorized("Invalid webhook signature.".into()));
    }

    let event: crate::bachs::WebhookEvent = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("Malformed webhook payload: {e}")))?;

    let Some(checkout_id) = event.data.checkout_id else {
        return Ok(axum::http::StatusCode::OK);
    };

    let Some(payment) = payment_by_bachs_session(&state, &checkout_id).await? else {
        tracing::warn!(checkout_id, "bachs webhook for unknown checkout session");
        return Ok(axum::http::StatusCode::OK);
    };

    match event.event_type.as_str() {
        "collection.succeeded" => {
            settle_payment(&state, &payment.transaction_id, &payment.id, Some(checkout_id), None).await?;
        }
        "collection.failed" | "collection.expired" if payment.status != "CONFIRMED" => {
            sqlx::query!(
                "UPDATE payments SET status = 'FAILED', failure_reason = $2, updated_at = now() WHERE id = $1",
                payment.id,
                event.event_type,
            )
            .execute(&state.db)
            .await?;

            let mut db_tx = state.db.begin().await?;
            // Best-effort: a webhook re-delivered after the transaction
            // moved on for some other reason shouldn't turn into a 500 that
            // makes Bachs retry indefinitely.
            let _ = apply_system_transition(
                &mut db_tx,
                &payment.transaction_id,
                TransactionStatus::PaymentFailed,
                &format!("Payment failed via Bachs webhook: {}", event.event_type),
            )
            .await;
            let _ = db_tx.commit().await;
        }
        _ => {}
    }

    Ok(axum::http::StatusCode::OK)
}

/// Marks the mock escrow payment as failed. See `mock_confirm_payment` for
/// why this needs to exist as its own endpoint rather than going through
/// the generic transition route.
pub async fn mock_fail_payment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<MockPaymentFailRequest>,
) -> AppResult<Json<TransactionWithHistory>> {
    let txn = load_transaction(&state, &id).await?;
    assert_is_buyer_on_txn(&auth, &txn)?;

    let payment = payment_for_txn(&state, &id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Call payment/initiate before reporting failure.".into()))?;

    let reason = body.reason.unwrap_or_else(|| "Payment failed.".to_string());

    sqlx::query!(
        "UPDATE payments SET status = 'FAILED', failure_reason = $2, updated_at = now() WHERE id = $1",
        payment.id,
        reason,
    )
    .execute(&state.db)
    .await?;

    let mut db_tx = state.db.begin().await?;
    let transaction = apply_system_transition(
        &mut db_tx,
        &id,
        TransactionStatus::PaymentFailed,
        &format!("Payment failed: {reason}"),
    )
    .await?;
    db_tx.commit().await?;

    let history = history_for(&state, &id).await?;
    Ok(Json(TransactionWithHistory { transaction, history }))
}

/// Reads the payment record for a transaction, if one exists yet.
pub async fn get_payment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Option<Payment>>> {
    let txn = load_transaction(&state, &id).await?;
    assert_participant_or_admin(&auth, &txn)?;
    Ok(Json(payment_for_txn(&state, &id).await?))
}
