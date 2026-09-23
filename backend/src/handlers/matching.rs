use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    ids, matching,
    models::{DemandRequest, Match, SupplyListing},
    services::{self, AuditParams},
    state::AppState,
};

/// Computes and persists matches for a demand against all active listings —
/// mirrors `matchingService.findMatchesForDemand`.
pub async fn find_for_demand(
    State(state): State<AppState>,
    user: AuthUser,
    Path(demand_id): Path<String>,
) -> AppResult<Json<Vec<Match>>> {
    let demand = sqlx::query_as::<_, DemandRequest>("SELECT * FROM demand_requests WHERE id = $1")
        .bind(&demand_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Demand not found.".to_string()))?;

    if demand.buyer_id != user.id && user.role != "admin" {
        return Err(AppError::Forbidden("You may only match your own demand.".to_string()));
    }

    let listings = sqlx::query_as::<_, SupplyListing>("SELECT * FROM supply_listings WHERE status = 'active'")
        .fetch_all(&state.db)
        .await?;

    let mut results = Vec::new();
    for listing in &listings {
        if listing.supplier_id == demand.buyer_id {
            continue; // can't buy from yourself
        }
        let Some((score, factors)) = matching::compute_match(&demand, listing) else { continue };
        if score < matching::MIN_SCORE {
            continue;
        }

        let id = ids::match_id(&demand.id, &listing.id);
        let m = sqlx::query_as::<_, Match>(
            r#"INSERT INTO matches (id, demand_id, listing_id, buyer_id, supplier_id, score, factors)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (demand_id, listing_id) DO UPDATE SET score = EXCLUDED.score, factors = EXCLUDED.factors
               RETURNING *"#,
        )
        .bind(&id)
        .bind(&demand.id)
        .bind(&listing.id)
        .bind(&demand.buyer_id)
        .bind(&listing.supplier_id)
        .bind(score)
        .bind(sqlx::types::Json(factors))
        .fetch_one(&state.db)
        .await?;
        results.push(m);
    }
    results.sort_by(|a, b| b.score.cmp(&a.score));

    if !results.is_empty() {
        services::log_audit(&state.db, AuditParams {
            action: "match_generated",
            actor_id: "system",
            actor_name: "AgriFlow System",
            actor_role: "system",
            entity_id: Some(&demand.id),
            entity_type: Some("Match"),
            detail: Some(&format!("{} match(es) generated for demand {}.", results.len(), demand.id)),
            transaction_id: None,
        }).await?;
    }

    Ok(Json(results))
}

pub async fn for_listing(State(state): State<AppState>, Path(listing_id): Path<String>) -> AppResult<Json<Vec<Match>>> {
    let rows = sqlx::query_as::<_, Match>("SELECT * FROM matches WHERE listing_id = $1 ORDER BY score DESC")
        .bind(&listing_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows))
}
