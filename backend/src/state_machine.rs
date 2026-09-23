//! Transaction state machine. Ported 1:1 from the frontend's
//! `src/services/transactionStateMachine.ts` so backend and UI never disagree
//! about what transitions are legal.

pub fn valid_transitions(from: &str) -> &'static [&'static str] {
    match from {
        "PENDING" => &["PENDING_SUPPLIER_ACCEPTANCE", "ACCEPTED", "REJECTED", "CANCELLED"],
        "PENDING_SUPPLIER_ACCEPTANCE" => &["ACCEPTED", "REJECTED", "CANCELLED"],
        "ACCEPTED" => &["PAYMENT_PENDING", "CANCELLED"],
        "REJECTED" => &[],
        "PAYMENT_PENDING" => &["PAYMENT_CONFIRMED", "PAYMENT_FAILED", "PAYMENT_CANCELLED"],
        "PAYMENT_CONFIRMED" => &["LOGISTICS_PENDING", "LOGISTICS_ASSIGNED", "LOGISTICS_ACCEPTED", "READY_FOR_PICKUP"],
        "PAYMENT_FAILED" => &["PAYMENT_PENDING", "CANCELLED"],
        "PAYMENT_CANCELLED" => &["CANCELLED"],
        "LOGISTICS_PENDING" => &["LOGISTICS_ASSIGNED", "LOGISTICS_ACCEPTED", "READY_FOR_PICKUP"],
        "LOGISTICS_ASSIGNED" => &["LOGISTICS_ACCEPTED", "LOGISTICS_REJECTED", "READY_FOR_PICKUP"],
        "LOGISTICS_ACCEPTED" => &["READY_FOR_PICKUP", "PICKED_UP"],
        "LOGISTICS_REJECTED" => &["LOGISTICS_PENDING"],
        "READY_FOR_PICKUP" => &["PICKED_UP", "IN_TRANSIT"],
        "PICKED_UP" => &["IN_TRANSIT", "DELIVERED"],
        "IN_TRANSIT" => &["DELIVERED", "BUYER_CONFIRMATION_PENDING", "DELIVERY_FAILED"],
        "DELIVERED" => &["BUYER_CONFIRMATION_PENDING", "DELIVERY_CONFIRMED", "COMPLETED", "DISPUTED"],
        "BUYER_CONFIRMATION_PENDING" => &["COMPLETED", "DISPUTED"],
        "DELIVERY_CONFIRMED" => &["COMPLETED"],
        "DELIVERY_FAILED" => &["DISPUTED"],
        "COMPLETED" => &[],
        "CANCELLED" => &[],
        "DISPUTED" => &["COMPLETED", "CANCELLED"],
        _ => &[],
    }
}

fn allowed_actors(to: &str) -> Option<&'static [&'static str]> {
    match to {
        "PENDING_SUPPLIER_ACCEPTANCE" => Some(&["buyer", "system"]),
        "ACCEPTED" => Some(&["supplier"]),
        "REJECTED" => Some(&["supplier"]),
        "PAYMENT_PENDING" => Some(&["buyer"]),
        "PAYMENT_CONFIRMED" => Some(&["system"]),
        "PAYMENT_FAILED" => Some(&["system"]),
        "PAYMENT_CANCELLED" => Some(&["buyer"]),
        "LOGISTICS_PENDING" => Some(&["system"]),
        "LOGISTICS_ASSIGNED" => Some(&["admin"]),
        "LOGISTICS_ACCEPTED" => Some(&["logistics"]),
        "LOGISTICS_REJECTED" => Some(&["logistics"]),
        "READY_FOR_PICKUP" => Some(&["logistics"]),
        "PICKED_UP" => Some(&["logistics"]),
        "IN_TRANSIT" => Some(&["logistics"]),
        "DELIVERED" => Some(&["logistics"]),
        "BUYER_CONFIRMATION_PENDING" => Some(&["logistics", "system"]),
        "DELIVERY_CONFIRMED" => Some(&["buyer"]),
        "DELIVERY_FAILED" => Some(&["buyer", "logistics"]),
        "COMPLETED" => Some(&["buyer", "system"]),
        "CANCELLED" => Some(&["buyer", "supplier", "admin"]),
        "DISPUTED" => Some(&["buyer"]),
        _ => None,
    }
}

pub fn can_transition(from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    if to == "ACCEPTED" && (from == "PAYMENT_PENDING" || from == "PAYMENT_CONFIRMED") {
        return true;
    }
    valid_transitions(from).contains(&to)
}

pub struct TransitionCheck {
    pub allowed: bool,
    pub reason: Option<String>,
}

pub fn can_actor_transition(from: &str, to: &str, actor_role: &str) -> TransitionCheck {
    if from == to {
        return TransitionCheck { allowed: true, reason: None };
    }
    if !can_transition(from, to) {
        return TransitionCheck {
            allowed: false,
            reason: Some(format!("Transition from {from} to {to} is not permitted.")),
        };
    }
    if let Some(allowed) = allowed_actors(to) {
        if !allowed.contains(&actor_role) {
            return TransitionCheck {
                allowed: false,
                reason: Some(format!("A {actor_role} cannot perform this action.")),
            };
        }
    }
    TransitionCheck { allowed: true, reason: None }
}
