mod auth;
mod bachs;
mod config;
mod email;
mod error;
mod ids;
mod models;
mod routes;
mod state;
mod state_machine;
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
    let bachs = bachs::BachsClient::new(
        config.bachs_secret_key.clone(),
        config.bachs_api_url.clone(),
        config.bachs_webhook_secret.clone(),
    );
    let state = AppState { db, config, mailer, bachs };
    let app = routes::build(state);

    tracing::info!("agriflow-api listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
