use sqlx::PgPool;

use crate::{error::{AppError, AppResult}, ids, models::Transaction, state_machine};

pub struct AuditParams<'a> {
    pub action: &'a str,
    pub actor_id: &'a str,
    pub actor_name: &'a str,
    pub actor_role: &'a str,
    pub entity_id: Option<&'a str>,
    pub entity_type: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub transaction_id: Option<&'a str>,
}

pub async fn log_audit(db: &PgPool, p: AuditParams<'_>) -> AppResult<()> {
    let id = ids::audit_id();
    sqlx::query(
        r#"INSERT INTO audit_events (id, action, actor_id, actor_name, actor_role, entity_id, entity_type, detail, transaction_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
    )
    .bind(&id)
    .bind(p.action)
    .bind(p.actor_id)
    .bind(p.actor_name)
    .bind(p.actor_role)
    .bind(p.entity_id)
    .bind(p.entity_type)
    .bind(p.detail)
    .bind(p.transaction_id)
    .execute(db)
    .await?;
    Ok(())
}

pub struct NotifyParams<'a> {
    pub user_id: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub message: &'a str,
    pub transaction_id: Option<&'a str>,
}

pub async fn notify(db: &PgPool, p: NotifyParams<'_>) -> AppResult<()> {
    let id = ids::notification_id();
    sqlx::query(
        r#"INSERT INTO notifications (id, user_id, type, title, message, transaction_id)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(&id)
    .bind(p.user_id)
    .bind(p.kind)
    .bind(p.title)
    .bind(p.message)
    .bind(p.transaction_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Well-known seeded admin id — mirrors PLATFORM_IDS.ADMIN_USER on the frontend.
/// Used as the fallback recipient for platform-wide operational notifications
/// (new logistics jobs pending assignment, disputes raised) when there is no
/// single admin to address.
pub const PLATFORM_ADMIN_ID: &str = "USR-ADM-001";

/// Drives a transaction through the state machine, recording the history
/// event, then fires the same audit-log + notification side effects the
/// frontend's `transactionService._emitSideEffects` performs. Ported so the
/// backend is the single source of truth once the UI talks to this API.
pub async fn transition_transaction(
    db: &PgPool,
    transaction_id: &str,
    to: &str,
    actor_id: &str,
    actor_name: &str,
    actor_role: &str,
    note: Option<&str>,
) -> AppResult<Transaction> {
    let txn = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = $1")
        .bind(transaction_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("Transaction not found.".to_string()))?;

    let check = state_machine::can_actor_transition(&txn.status, to, actor_role);
    if !check.allowed {
        return Err(AppError::Conflict(check.reason.unwrap_or_else(|| "Transition not allowed.".to_string())));
    }

    let updated = sqlx::query_as::<_, Transaction>(
        "UPDATE transactions SET status = $1, updated_at = now() WHERE id = $2 RETURNING *",
    )
    .bind(to)
    .bind(transaction_id)
    .fetch_one(db)
    .await?;

    sqlx::query(
        "INSERT INTO transaction_events (transaction_id, status, actor, actor_role, note) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(transaction_id)
    .bind(to)
    .bind(actor_name)
    .bind(actor_role)
    .bind(note)
    .execute(db)
    .await?;

    emit_transition_side_effects(db, &updated, actor_id, actor_name, actor_role).await?;

    Ok(updated)
}

async fn emit_transition_side_effects(
    db: &PgPool,
    updated: &Transaction,
    actor_id: &str,
    actor_name: &str,
    actor_role: &str,
) -> AppResult<()> {
    let id = updated.id.as_str();
    let status = updated.status.as_str();

    let audit: Option<(&str, Option<&str>, Option<&str>, String)> = match status {
        "ACCEPTED" => Some(("transaction_accepted", Some(id), Some("Transaction"), format!("Supplier accepted transaction {id}."))),
        "REJECTED" => Some(("transaction_rejected", Some(id), Some("Transaction"), format!("Supplier rejected transaction {id}."))),
        "PAYMENT_CONFIRMED" => Some(("payment_confirmed", Some(id), Some("Transaction"), format!("Payment confirmed for transaction {id}."))),
        "PAYMENT_FAILED" => Some(("payment_failed", Some(id), Some("Transaction"), format!("Payment failed for transaction {id}."))),
        "LOGISTICS_PENDING" => Some(("logistics_job_created", Some(id), Some("LogisticsJob"), format!("Logistics job created for transaction {id}."))),
        "LOGISTICS_ASSIGNED" => Some(("logistics_provider_assigned", Some(id), Some("LogisticsJob"), format!("Logistics provider assigned to transaction {id}."))),
        "LOGISTICS_ACCEPTED" => Some(("logistics_job_accepted", Some(id), Some("LogisticsJob"), format!("{actor_name} accepted logistics job for transaction {id}."))),
        "READY_FOR_PICKUP" => Some(("shipment_ready", Some(id), Some("LogisticsJob"), format!("Shipment ready for pickup. Transaction {id}."))),
        "PICKED_UP" => Some(("shipment_picked_up", Some(id), Some("LogisticsJob"), format!("Shipment picked up from {}. Transaction {id}.", updated.pickup_location))),
        "IN_TRANSIT" => Some(("shipment_in_transit", Some(id), Some("LogisticsJob"), format!("Shipment in transit to {}. Transaction {id}.", updated.delivery_location))),
        "DELIVERED" => Some(("shipment_delivered", Some(id), Some("LogisticsJob"), format!("Shipment delivered at {}. Transaction {id}.", updated.delivery_location))),
        "DELIVERY_CONFIRMED" => Some(("delivery_confirmed", Some(id), Some("Transaction"), format!("Buyer confirmed delivery for transaction {id}."))),
        "COMPLETED" => Some(("transaction_completed", Some(id), Some("Transaction"), format!("Transaction {id} completed successfully."))),
        _ => None,
    };

    if let Some((action, entity_id, entity_type, detail)) = audit {
        log_audit(db, AuditParams {
            action,
            actor_id,
            actor_name,
            actor_role,
            entity_id,
            entity_type,
            detail: Some(&detail),
            transaction_id: Some(id),
        }).await?;
    }

    match status {
        "ACCEPTED" => {
            notify(db, NotifyParams {
                user_id: &updated.buyer_id,
                kind: "transaction_accepted",
                title: "Transaction Accepted",
                message: &format!("Your transaction {id} has been accepted by {}. Please proceed with payment.", updated.supplier_name),
                transaction_id: Some(id),
            }).await?;
        }
        "REJECTED" => {
            notify(db, NotifyParams {
                user_id: &updated.buyer_id,
                kind: "transaction_rejected",
                title: "Transaction Rejected",
                message: &format!("Your transaction request {id} was rejected by the supplier."),
                transaction_id: Some(id),
            }).await?;
        }
        "PAYMENT_CONFIRMED" => {
            notify(db, NotifyParams {
                user_id: &updated.buyer_id,
                kind: "payment_confirmed",
                title: "Payment Confirmed",
                message: &format!("Payment for transaction {id} has been confirmed. A logistics job has been created."),
                transaction_id: Some(id),
            }).await?;
            notify(db, NotifyParams {
                user_id: PLATFORM_ADMIN_ID,
                kind: "logistics_assigned",
                title: "New Logistics Job Pending",
                message: &format!("Transaction {id} requires a logistics provider assignment."),
                transaction_id: Some(id),
            }).await?;
        }
        "DELIVERED" => {
            notify(db, NotifyParams {
                user_id: &updated.buyer_id,
                kind: "delivery_received",
                title: "Shipment Delivered",
                message: &format!("Your shipment for transaction {id} has been marked delivered. Please confirm receipt."),
                transaction_id: Some(id),
            }).await?;
        }
        "DELIVERY_CONFIRMED" => {
            notify(db, NotifyParams {
                user_id: &updated.supplier_id,
                kind: "delivery_confirmed",
                title: "Delivery Confirmed",
                message: &format!("The buyer has confirmed delivery for transaction {id}. Transaction is now complete."),
                transaction_id: Some(id),
            }).await?;
        }
        "COMPLETED" => {
            notify(db, NotifyParams {
                user_id: &updated.buyer_id,
                kind: "transaction_completed",
                title: "Transaction Complete",
                message: &format!("Transaction {id} has been marked complete."),
                transaction_id: Some(id),
            }).await?;
        }
        _ => {}
    }

    Ok(())
}
