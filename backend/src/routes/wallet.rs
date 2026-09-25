//! Supplier/logistics earnings balance + payout requests (issue #39).
//! Ported 1:1 from `src/services/walletService.ts`.
//!
//! No real payout rail is integrated here, same as every other "mock"
//! endpoint in this backend (see `mock_confirm_payment`'s doc comment) --
//! there's no NIBSS or on-chain minting call to make, since none exists in
//! this codebase to call. `payout_tx_hash` is a generated placeholder
//! reference, matching the frontend's own fallback format exactly (it also
//! falls back to a placeholder whenever its real on-chain mint attempt
//! fails). Real on-chain payouts belong with the Web3 relayer/indexer work
//! (issues #54-56), not reimplemented here.

use axum::{Json, extract::State};
use chrono::Utc;
use rust_decimal::Decimal;

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::user::UserRole;
use crate::models::wallet::{WalletSummary, Withdrawal, WithdrawRequest};
use crate::state::AppState;
use crate::validation::require_non_empty;

async fn earned_and_pending(state: &AppState, auth: &AuthUser) -> AppResult<(Decimal, Decimal, i64)> {
    match auth.role {
        UserRole::Supplier => {
            let row = sqlx::query!(
                r#"
                SELECT
                    COALESCE(SUM(total_amount) FILTER (WHERE status = 'COMPLETED'), 0) AS "total_earned!",
                    COALESCE(SUM(total_amount) FILTER (WHERE status IN (
                        'PAYMENT_CONFIRMED', 'LOGISTICS_PENDING', 'LOGISTICS_ASSIGNED', 'LOGISTICS_ACCEPTED',
                        'READY_FOR_PICKUP', 'PICKED_UP', 'IN_TRANSIT', 'DELIVERED',
                        'BUYER_CONFIRMATION_PENDING', 'DELIVERY_CONFIRMED'
                    )), 0) AS "pending_escrow!",
                    COUNT(*) FILTER (WHERE status = 'COMPLETED') AS "completed_count!"
                FROM transactions WHERE supplier_id = $1
                "#,
                auth.user_id,
            )
            .fetch_one(&state.db)
            .await?;
            Ok((row.total_earned, row.pending_escrow, row.completed_count))
        }
        UserRole::Logistics => {
            let row = sqlx::query!(
                r#"
                SELECT
                    COALESCE(SUM(logistics_cost) FILTER (WHERE status = 'COMPLETED'), 0) AS "total_earned!",
                    COALESCE(SUM(logistics_cost) FILTER (WHERE status IN (
                        'ASSIGNED', 'ACCEPTED', 'READY_FOR_PICKUP', 'PICKED_UP', 'IN_TRANSIT', 'DELIVERED'
                    )), 0) AS "pending_escrow!",
                    COUNT(*) FILTER (WHERE status = 'COMPLETED') AS "completed_count!"
                FROM logistics_jobs WHERE provider_id = $1
                "#,
                auth.user_id,
            )
            .fetch_one(&state.db)
            .await?;
            Ok((row.total_earned, row.pending_escrow, row.completed_count))
        }
        _ => unreachable!("gated by require_any_role in callers"),
    }
}

async fn withdrawn_total(state: &AppState, user_id: &str) -> AppResult<Decimal> {
    Ok(sqlx::query_scalar!(
        r#"SELECT COALESCE(SUM(amount), 0) AS "total!" FROM withdrawals WHERE user_id = $1 AND status = 'COMPLETED'"#,
        user_id,
    )
    .fetch_one(&state.db)
    .await?)
}

pub async fn summary(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<WalletSummary>> {
    auth.require_any_role(&[UserRole::Supplier, UserRole::Logistics])?;

    let (total_earned, pending_escrow, completed_count) = earned_and_pending(&state, &auth).await?;
    let withdrawn = withdrawn_total(&state, &auth.user_id).await?;
    let available = (total_earned - withdrawn).max(Decimal::ZERO);

    Ok(Json(WalletSummary {
        total_earned,
        pending_escrow,
        withdrawn,
        available,
        currency: "NGN".into(),
        completed_count,
    }))
}

pub async fn withdraw(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<WithdrawRequest>,
) -> AppResult<Json<Withdrawal>> {
    auth.require_any_role(&[UserRole::Supplier, UserRole::Logistics])?;

    if body.amount <= Decimal::ZERO {
        return Err(AppError::BadRequest("Withdrawal amount must be greater than zero.".into()));
    }

    // 'stellar_usdc' accepted as a legacy synonym, normalized the same way
    // the frontend does -- never stored as its own value (see migration).
    let method = match body.method.as_str() {
        "bank_transfer" => "bank_transfer",
        "crypto_usdc" | "stellar_usdc" => "crypto_usdc",
        other => return Err(AppError::BadRequest(format!("Unknown withdrawal method: {other}"))),
    };

    let (bank_name, account_number, account_name, stellar_public_key) = if method == "bank_transfer" {
        let details = body
            .bank_details
            .ok_or_else(|| AppError::BadRequest("bankDetails is required for bank_transfer.".into()))?;
        require_non_empty("bankDetails.bankName", &details.bank_name)?;
        require_non_empty("bankDetails.accountNumber", &details.account_number)?;
        require_non_empty("bankDetails.accountName", &details.account_name)?;
        (Some(details.bank_name), Some(details.account_number), Some(details.account_name), None)
    } else {
        let key = body
            .stellar_public_key
            .ok_or_else(|| AppError::BadRequest("stellarPublicKey is required for crypto_usdc.".into()))?;
        require_non_empty("stellarPublicKey", &key)?;
        (None, None, None, Some(key))
    };

    let mut db_tx = state.db.begin().await?;

    // Serializes concurrent withdrawal requests from the same user within
    // this transaction, so two simultaneous requests can't both read the
    // same `available` balance and jointly overdraw it.
    sqlx::query!("SELECT pg_advisory_xact_lock(hashtext($1))", auth.user_id)
        .execute(&mut *db_tx)
        .await?;

    let (total_earned, _pending, _count) = earned_and_pending(&state, &auth).await?;
    let withdrawn = withdrawn_total(&state, &auth.user_id).await?;
    let available = (total_earned - withdrawn).max(Decimal::ZERO);

    if body.amount > available {
        return Err(AppError::BadRequest(format!(
            "Insufficient available escrow earnings. Available: {available}, Requested: {}",
            body.amount
        )));
    }

    let id = crate::ids::generate("WTH-AGF");
    let now_millis = Utc::now().timestamp_millis();
    let payout_tx_hash = if method == "crypto_usdc" {
        format!("0x_payout_{now_millis:x}")
    } else {
        format!("NIBSS_PAY_{:08}", now_millis % 100_000_000)
    };
    let role_str = auth.role.to_string();

    let withdrawal = sqlx::query_as!(
        Withdrawal,
        r#"
        INSERT INTO withdrawals
            (id, user_id, user_name, user_role, amount, currency, method,
             bank_name, account_number, account_name, stellar_public_key, status, payout_tx_hash)
        VALUES ($1, $2, $3, $4, $5, 'NGN', $6, $7, $8, $9, $10, 'COMPLETED', $11)
        RETURNING id, user_id, user_name, user_role, amount, currency, method,
                  bank_name, account_number, account_name, stellar_public_key, status, payout_tx_hash, created_at
        "#,
        id,
        auth.user_id,
        auth.name,
        role_str,
        body.amount,
        method,
        bank_name,
        account_number,
        account_name,
        stellar_public_key,
        payout_tx_hash,
    )
    .fetch_one(&mut *db_tx)
    .await?;

    db_tx.commit().await?;

    Ok(Json(withdrawal))
}
