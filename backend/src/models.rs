use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ─── User & Auth ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub name: String,
    pub role: String,
    pub organization_name: Option<String>,
    pub phone: Option<String>,
    pub location: Option<String>,
    pub verified: bool,
    pub profile_complete: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthSession {
    pub token: String,
    pub user_id: String,
    pub role: String,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: String,
    pub organization_name: Option<String>,
    pub phone: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub organization_name: Option<String>,
    pub phone: Option<String>,
    pub location: Option<String>,
}

// ─── Supply Listing ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SupplyListing {
    pub id: String,
    pub supplier_id: String,
    pub supplier_name: String,
    pub supplier_verified: bool,
    pub commodity: String,
    pub quantity: f64,
    pub unit: String,
    pub quality_grade: String,
    pub price_per_unit: f64,
    pub currency: String,
    pub location: String,
    pub availability_date: DateTime<Utc>,
    pub description: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSupplyRequest {
    pub commodity: String,
    pub quantity: f64,
    pub unit: String,
    pub quality_grade: String,
    pub price_per_unit: f64,
    pub currency: String,
    pub location: String,
    pub availability_date: DateTime<Utc>,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSupplyRequest {
    pub quantity: Option<f64>,
    pub price_per_unit: Option<f64>,
    pub description: Option<String>,
    pub status: Option<String>,
}

// ─── Demand Request ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct DemandRequest {
    pub id: String,
    pub buyer_id: String,
    pub buyer_name: String,
    pub commodity: String,
    pub quantity: f64,
    pub unit: String,
    pub quality_grade: String,
    pub destination_location: String,
    pub required_by_date: DateTime<Utc>,
    pub indicative_budget: f64,
    pub currency: String,
    pub notes: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDemandRequest {
    pub commodity: String,
    pub quantity: f64,
    pub unit: String,
    pub quality_grade: String,
    pub destination_location: String,
    pub required_by_date: DateTime<Utc>,
    pub indicative_budget: f64,
    pub currency: String,
    pub notes: Option<String>,
}

// ─── Match ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchFactor {
    pub label: String,
    pub matched: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Match {
    pub id: String,
    pub demand_id: String,
    pub listing_id: String,
    pub buyer_id: String,
    pub supplier_id: String,
    pub score: i32,
    pub factors: sqlx::types::Json<Vec<MatchFactor>>,
    pub created_at: DateTime<Utc>,
}

// ─── Transaction ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Transaction {
    pub id: String,
    pub listing_id: String,
    pub demand_id: Option<String>,
    pub buyer_id: String,
    pub buyer_name: String,
    pub supplier_id: String,
    pub supplier_name: String,
    pub commodity: String,
    pub quantity: f64,
    pub unit: String,
    pub quality_grade: String,
    pub price_per_unit: f64,
    pub total_amount: f64,
    pub currency: String,
    pub pickup_location: String,
    pub delivery_location: String,
    pub expected_delivery_date: DateTime<Utc>,
    pub status: String,
    pub payment_id: Option<String>,
    pub logistics_job_id: Option<String>,
    pub dispute_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TransactionEvent {
    pub status: String,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub actor_role: String,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TransactionWithHistory {
    #[serde(flatten)]
    pub transaction: Transaction,
    pub history: Vec<TransactionEvent>,
}

#[derive(Debug, Deserialize)]
pub struct InitiateTransactionRequest {
    pub listing_id: String,
    pub quantity: f64,
    pub delivery_location: String,
    pub expected_delivery_date: DateTime<Utc>,
    pub demand_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionRequest {
    pub to: String,
    pub note: Option<String>,
}

// ─── Payment ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Payment {
    pub id: String,
    pub transaction_id: String,
    pub payer_id: String,
    pub amount: f64,
    pub currency: String,
    pub provider: String,
    pub provider_reference: Option<String>,
    pub status: String,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct InitiatePaymentRequest {
    pub transaction_id: String,
}

#[derive(Debug, Deserialize)]
pub struct FailPaymentRequest {
    pub reason: String,
}

// ─── Logistics ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct LogisticsJob {
    pub id: String,
    pub transaction_id: String,
    pub commodity: String,
    pub quantity: f64,
    pub unit: String,
    pub pickup_location: String,
    pub delivery_location: String,
    pub pickup_date: DateTime<Utc>,
    pub expected_delivery_date: DateTime<Utc>,
    pub logistics_cost: f64,
    pub currency: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub status: String,
    pub proof_recipient_name: Option<String>,
    pub proof_delivery_note: Option<String>,
    pub proof_timestamp: Option<DateTime<Utc>>,
    pub proof_recorded_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AssignProviderRequest {
    pub provider_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ProofOfDeliveryInput {
    pub recipient_name: String,
    pub delivery_note: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJobStatusRequest {
    pub status: String,
    pub proof_of_delivery: Option<ProofOfDeliveryInput>,
}

#[derive(Debug, Deserialize)]
pub struct RejectJobRequest {
    pub reason: String,
}

// ─── Dispute ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Dispute {
    pub id: String,
    pub transaction_id: String,
    pub raised_by_id: String,
    pub raised_by_name: String,
    pub reason: String,
    pub description: String,
    pub status: String,
    pub resolution: Option<String>,
    pub resolved_by_id: Option<String>,
    pub resolved_by_name: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RaiseDisputeRequest {
    pub transaction_id: String,
    pub reason: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveDisputeRequest {
    pub decision: String,
    /// "completed" | "cancelled"
    pub outcome: String,
}

// ─── Notification ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Notification {
    pub id: String,
    pub user_id: String,
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub message: String,
    pub transaction_id: Option<String>,
    pub read: bool,
    pub created_at: DateTime<Utc>,
}

// ─── Audit ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AuditEvent {
    pub id: String,
    pub action: String,
    pub actor_id: String,
    pub actor_name: String,
    pub actor_role: String,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub detail: Option<String>,
    pub transaction_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
