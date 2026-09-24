pub mod auth;
pub mod demands;
pub mod listings;
pub mod transactions;

use axum::{Router, routing::{get, post}};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::state::AppState;

/// Catches any request that matched no route at all, so an unmatched path
/// returns the same `{"error": "..."}` shape as everything else instead of
/// Axum's bare empty-body 404.
async fn not_found() -> AppError {
    AppError::NotFound("The requested resource was not found.".to_string())
}

pub fn build(state: AppState) -> Router {
    let api = Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/me", get(auth::me))
        .route("/listings", get(listings::list_active).post(listings::create))
        .route("/listings/mine", get(listings::mine))
        .route("/listings/{id}", get(listings::get_one).patch(listings::update))
        .route("/demands", get(demands::list_open).post(demands::create))
        .route("/demands/mine", get(demands::mine))
        .route("/demands/{id}", get(demands::get_one))
        .route("/transactions", get(transactions::list_mine).post(transactions::create))
        .route("/transactions/{id}", get(transactions::get_one))
        .route("/transactions/{id}/transition", post(transactions::transition))
        .route("/transactions/{id}/payment/confirm", post(transactions::mock_confirm_payment))
        .route("/transactions/{id}/payment/fail", post(transactions::mock_fail_payment))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .fallback(not_found)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
