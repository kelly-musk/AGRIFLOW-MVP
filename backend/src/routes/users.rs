//! Self-service wallet address linking (prerequisite for issue #54 --
//! `AgriFlowEscrow.fundTradeFromIntent` needs real addresses for
//! buyer/supplier/logistics, and nothing mapped a platform account to one
//! before this).
//!
//! Deliberately self-attested, not cryptographically verified (e.g. no
//! signed-nonce proof of key ownership): a user can only ever set their
//! own `wallet_address`, never someone else's, so a wrong or unowned
//! address only ever costs that same user -- they can't redirect funds
//! meant for anyone else. Verifying real ownership would mean the wallet
//! signing a challenge, which needs frontend wallet-connect support that
//! doesn't exist yet; this is the honest floor above "trust nothing" and
//! below "cryptographically proven", same trust level as this backend's
//! other self-attested flows (see mock_confirm_payment's doc comment).

use axum::{Json, extract::State};

use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::models::user::{SetWalletAddressRequest, User, UserPublic};
use crate::state::AppState;
use crate::validation::is_valid_evm_address;

pub async fn set_wallet_address(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetWalletAddressRequest>,
) -> AppResult<Json<UserPublic>> {
    if !is_valid_evm_address(&body.wallet_address) {
        return Err(AppError::BadRequest(
            "walletAddress must be a 0x-prefixed 40-character hex EVM address.".into(),
        ));
    }

    let user = sqlx::query_as!(
        User,
        r#"
        UPDATE users SET wallet_address = $2, updated_at = now() WHERE id = $1
        RETURNING id, email, password_hash, name, role as "role: _", organization_name, phone, location, verified, profile_complete, wallet_address, created_at, updated_at
        "#,
        auth.user_id,
        body.wallet_address,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.is_unique_violation() {
                return AppError::Conflict("This wallet address is already linked to another account.".into());
            }
        }
        AppError::from(e)
    })?
    .ok_or_else(|| AppError::NotFound("User not found.".into()))?;

    Ok(Json(UserPublic::from(user)))
}
