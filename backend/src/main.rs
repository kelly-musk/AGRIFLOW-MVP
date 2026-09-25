mod auth;
mod bachs;
mod chain;
mod config;
mod email;
mod error;
mod ids;
mod media;
mod models;
mod routes;
mod state;
mod state_machine;
mod storage;
mod validation;

use sqlx::postgres::PgPoolOptions;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env()?;
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    let addr = config.server_addr;
    let mailer = email::Mailer::new(config.resend_api_key.clone(), config.email_from.clone());
    if config.bachs_secret_key.is_none() {
        tracing::warn!("BACHS_SECRET_KEY is not set — Bachs checkout is disabled");
    }
    if config.bachs_webhook_secret.is_none() {
        tracing::warn!("BACHS_WEBHOOK_SECRET is not set — Bachs webhooks will be rejected");
    }
    let http = reqwest::Client::new();
    let storage = match &config.storage {
        Some(cfg) => Some(storage::Storage::new(cfg, http.clone())?),
        None => {
            tracing::warn!("S3_* is not set — listing media uploads are disabled");
            None
        }
    };
    let state = AppState {
        db,
        config,
        mailer,
        http,
        storage,
        // ffmpeg is CPU- and memory-heavy; two at a time keeps a small
        // instance responsive.
        media_jobs: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
    };
    if let Some(storage) = &state.storage {
        if state.config.storage_cors_origins.is_empty() {
            tracing::warn!("no FRONTEND_BASE_URL / S3_CORS_ORIGINS — browsers can't upload to the media bucket");
        } else if let Err(e) = storage.put_cors(&state.config.storage_cors_origins).await {
            // Not fatal: uploads from the browser fail until it's fixed,
            // everything else keeps working.
            tracing::error!(error = %format!("{e:#}"), "could not set media bucket CORS");
        } else {
            tracing::info!(origins = ?state.config.storage_cors_origins, "media bucket CORS set");
        }
        media::resume_processing(&state).await;
        media::spawn_cleanup(state.clone());
    }
    chain::spawn_indexer(state.clone());
    let app = routes::build(state);

    tracing::info!("agriflow-api listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
