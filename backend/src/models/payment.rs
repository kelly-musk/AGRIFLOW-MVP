use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Payment {
    pub id: String,
    pub transaction_id: String,
    pub payer_id: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub amount: Decimal,
    pub currency: String,
    pub provider: String,
    pub provider_reference: Option<String>,
    pub stellar_tx_hash: Option<String>,
    pub bachs_session_id: Option<String>,
    pub status: String,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitiatePaymentRequest {
    pub amount: Decimal,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmPaymentRequest {
    pub stellar_tx_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBachsSessionRequest {
    /// The frontend's own origin (e.g. `window.location.origin`), used to
    /// build the success/cancel redirect URLs. Only ever used as a redirect
    /// target -- the redirect itself carries no authority in this design,
    /// since payment confirmation only ever happens via the verified
    /// webhook, never the redirect.
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBachsSessionResponse {
    pub checkout_url: String,
}
