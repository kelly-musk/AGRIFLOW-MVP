use axum::{
    Json,
    extract::{Path, State},
};
use std::str::FromStr;

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::ids;
use crate::json_extractor::AppJson;
use crate::models::listing::SupplyListing;
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
    AppJson(body): AppJson<CreateTransactionRequest>,
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
    AppJson(body): AppJson<TransitionRequest>,
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

/// Settles the mock escrow payment for a transaction the buyer initiated,
/// then immediately queues it for logistics.
///
/// There is no real payment provider integrated yet (see backend/README.md
/// "Not built yet" — escrow/payments is the next slice) and no service/
/// webhook credential mechanism exists either, so the state machine's
/// `PAYMENT_CONFIRMED`/`LOGISTICS_PENDING` transitions — which require
/// `Actor::System` — could not be reached by any real caller: JWT roles map
/// only to Buyer/Supplier/Logistics/Admin, never System. This endpoint is
/// the one legitimate place that gap is bridged: it's gated to exactly the
/// buyer who owns the transaction, only from `PAYMENT_PENDING`, and it
/// performs the *same* system-actor transitions a real payment webhook
/// would trigger once one exists — it does not loosen the state machine's
/// actor table itself, and POST /transactions/:id/transition still rejects
/// these on any role, System included, since nothing can present System's
/// credentials there.
pub async fn mock_confirm_payment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<TransactionWithHistory>> {
    let txn = load_transaction(&state, &id).await?;
    assert_is_buyer_on_txn(&auth, &txn)?;

    let mut db_tx = state.db.begin().await?;
    apply_system_transition(
        &mut db_tx,
        &id,
        TransactionStatus::PaymentConfirmed,
        "Payment confirmed (mock escrow).",
    )
    .await?;
    let transaction = apply_system_transition(
        &mut db_tx,
        &id,
        TransactionStatus::LogisticsPending,
        "Logistics job queued.",
    )
    .await?;
    db_tx.commit().await?;

    let history = history_for(&state, &id).await?;
    Ok(Json(TransactionWithHistory { transaction, history }))
}

/// Marks the mock escrow payment as failed. See `mock_confirm_payment` for
/// why this needs to exist as its own endpoint rather than going through
/// the generic transition route.
pub async fn mock_fail_payment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<MockPaymentFailRequest>,
) -> AppResult<Json<TransactionWithHistory>> {
    let txn = load_transaction(&state, &id).await?;
    assert_is_buyer_on_txn(&auth, &txn)?;

    let reason = body.reason.unwrap_or_else(|| "Payment failed.".to_string());
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
