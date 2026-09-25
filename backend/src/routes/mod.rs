pub mod admin;
pub mod auth;
pub mod demands;
pub mod disputes;
pub mod health;
pub mod listings;
pub mod logistics;
pub mod media;
pub mod transactions;
pub mod wallet;
pub mod webhooks;

use axum::{Router, routing::{get, patch, post}};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn build(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health::health))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/admin/login", post(auth::admin_login))
        .route("/auth/me", get(auth::me))
        .route("/admin/users", get(admin::list_users))
        .route("/admin/users/{id}/verify", patch(admin::set_verified))
        .route("/listings", get(listings::list_active).post(listings::create))
        .route("/listings/mine", get(listings::mine))
        .route("/listings/{id}", get(listings::get_one).patch(listings::update))
        .route("/media/presigned-url", post(media::presigned_url))
        .route("/media/{id}", get(media::get_one).patch(media::update).delete(media::delete))
        .route("/media/{id}/complete", post(media::complete))
        .route("/media/{id}/content", get(media::content))
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
        .route("/webhooks/bachs", post(webhooks::bachs))
        .route("/logistics/jobs", get(logistics::list_jobs))
        .route("/logistics/jobs/{id}/claim", post(logistics::claim_job))
        .route("/logistics/jobs/{id}/assign", post(logistics::assign_job))
        .route("/logistics/jobs/{id}/status", patch(logistics::update_status))
        .route("/disputes", get(disputes::list).post(disputes::raise))
        .route("/disputes/{id}/resolve", post(disputes::resolve))
        .route("/wallet/summary", get(wallet::summary))
        .route("/wallet/withdraw", post(wallet::withdraw))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
