use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Withdrawal {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub user_role: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub amount: Decimal,
    pub currency: String,
    pub method: String,
    pub bank_name: Option<String>,
    pub account_number: Option<String>,
    pub account_name: Option<String>,
    pub stellar_public_key: Option<String>,
    pub status: String,
    pub payout_tx_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletSummary {
    #[serde(with = "rust_decimal::serde::float")]
    pub total_earned: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub pending_escrow: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub withdrawn: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub available: Decimal,
    pub currency: String,
    pub completed_count: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BankDetails {
    pub bank_name: String,
    pub account_number: String,
    pub account_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawRequest {
    pub amount: Decimal,
    pub method: String,
    pub bank_details: Option<BankDetails>,
    pub stellar_public_key: Option<String>,
}
