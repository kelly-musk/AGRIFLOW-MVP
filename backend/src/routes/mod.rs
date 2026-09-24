pub mod auth;
pub mod demands;
pub mod listings;
pub mod transactions;

use axum::{Router, routing::{get, post}};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

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
        .route("/transactions/{id}/payment", get(transactions::get_payment))
        .route("/transactions/{id}/payment/initiate", post(transactions::initiate_payment))
        .route("/transactions/{id}/payment/confirm", post(transactions::mock_confirm_payment))
        .route("/transactions/{id}/payment/fail", post(transactions::mock_fail_payment))
        .route(
            "/transactions/{id}/payment/bachs/checkout-session",
            post(transactions::create_bachs_checkout_session),
        )
        // Public: Bachs calls this directly, with no user session to present.
        // Protected instead by HMAC signature verification -- see bachs.rs.
        .route("/webhooks/bachs", post(transactions::bachs_webhook))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
