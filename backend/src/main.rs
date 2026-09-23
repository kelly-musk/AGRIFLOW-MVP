mod auth;
mod error;
mod handlers;
mod ids;
mod matching;
mod models;
mod seed;
mod services;
mod state;
mod state_machine;

use axum::{
    Router,
    routing::{get, patch, post},
};
use sqlx::postgres::PgPoolOptions;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "agriflow_api=info,tower_http=info".into()))
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://agriflow:agriflow_dev_pw@localhost:5433/agriflow".to_string());
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("JWT_SECRET not set — using an insecure default. Set it in production.");
        "dev-insecure-secret-change-me".to_string()
    });
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;
    seed::run(&db).await?;

    let state = AppState { db, jwt_secret };

    let app = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        // auth
        .route("/api/auth/register", post(handlers::auth::register))
        .route("/api/auth/login", post(handlers::auth::login))
        .route("/api/auth/me", get(handlers::auth::me))
        .route("/api/auth/profile", patch(handlers::auth::update_profile))
        // supply
        .route("/api/supply", post(handlers::supply::create).get(handlers::supply::list_active))
        .route("/api/supply/mine", get(handlers::supply::mine))
        .route("/api/supply/{id}", get(handlers::supply::get_one).patch(handlers::supply::update))
        .route("/api/supply/{id}/matches", get(handlers::matching::for_listing))
        // demand
        .route("/api/demand", post(handlers::demand::create).get(handlers::demand::list_all))
        .route("/api/demand/mine", get(handlers::demand::mine))
        .route("/api/demand/{id}", get(handlers::demand::get_one))
        .route("/api/demand/{id}/matches", post(handlers::matching::find_for_demand))
        // transactions
        .route("/api/transactions", post(handlers::transactions::create).get(handlers::transactions::list_mine))
        .route("/api/transactions/{id}", get(handlers::transactions::get_one))
        .route("/api/transactions/{id}/transition", post(handlers::transactions::transition))
        // payments
        .route("/api/payments/initiate", post(handlers::payments::initiate))
        .route("/api/payments/{id}/confirm", post(handlers::payments::confirm))
        .route("/api/payments/{id}/fail", post(handlers::payments::fail))
        .route("/api/payments/transaction/{transaction_id}", get(handlers::payments::for_transaction))
        // logistics
        .route("/api/logistics/jobs/pending", get(handlers::logistics::pending))
        .route("/api/logistics/jobs/mine", get(handlers::logistics::mine))
        .route("/api/logistics/jobs/{id}", get(handlers::logistics::get_one))
        .route("/api/logistics/jobs/{id}/assign", post(handlers::logistics::assign))
        .route("/api/logistics/jobs/{id}/accept", post(handlers::logistics::accept))
        .route("/api/logistics/jobs/{id}/reject", post(handlers::logistics::reject))
        .route("/api/logistics/jobs/{id}/status", patch(handlers::logistics::set_status))
        .route("/api/logistics/jobs/transaction/{transaction_id}", get(handlers::logistics::for_transaction))
        .route("/api/logistics/providers", get(handlers::logistics::providers))
        // disputes
        .route("/api/disputes", post(handlers::disputes::raise).get(handlers::disputes::list_all))
        .route("/api/disputes/{id}", get(handlers::disputes::get_one))
        .route("/api/disputes/{id}/resolve", post(handlers::disputes::resolve))
        // notifications
        .route("/api/notifications", get(handlers::notifications::mine))
        .route("/api/notifications/unread-count", get(handlers::notifications::unread_count))
        .route("/api/notifications/{id}/read", post(handlers::notifications::mark_read))
        .route("/api/notifications/read-all", post(handlers::notifications::mark_all_read))
        // audit
        .route("/api/audit", get(handlers::audit::list_all))
        .route("/api/audit/transaction/{transaction_id}", get(handlers::audit::for_transaction))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("agriflow-api listening on :{port}");
    axum::serve(listener, app).await?;

    Ok(())
}
