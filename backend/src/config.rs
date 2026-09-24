use std::net::SocketAddr;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_expiry_hours: i64,
    pub server_addr: SocketAddr,
    /// Shared secret that must be sent as `X-Admin-Registration-Key` to
    /// register an admin. Unset → admin self-registration is disabled.
    pub admin_registration_key: Option<String>,
    /// Resend API key. Unset → outgoing email is skipped (see `email.rs`).
    pub resend_api_key: Option<String>,
    /// Sender for outgoing email, e.g. `AgriFlow <hello@yourdomain.com>`.
    pub email_from: String,
    /// Bachs.io API secret key, sandbox base URL, and webhook signing
    /// secret. Defaults match the shared sandbox credentials already in
    /// `src/lib/bachs.ts` on the frontend -- these are intentionally
    /// reusable team credentials, not per-deployment secrets, per the
    /// maintainer. Override via env var if that ever changes.
    pub bachs_secret_key: String,
    pub bachs_api_url: String,
    pub bachs_webhook_secret: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is not set"))?;
        let jwt_secret =
            std::env::var("JWT_SECRET").map_err(|_| anyhow::anyhow!("JWT_SECRET is not set"))?;
        let jwt_expiry_hours = std::env::var("JWT_EXPIRY_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        // Railway (and most PaaS platforms) inject $PORT and expect the app
        // to bind to it; SERVER_ADDR remains the override for local/manual runs.
        let server_addr: SocketAddr = match std::env::var("PORT") {
            Ok(port) => format!("0.0.0.0:{port}").parse()?,
            Err(_) => std::env::var("SERVER_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
                .parse()?,
        };

        let non_empty = |name| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let admin_registration_key = non_empty("ADMIN_REGISTRATION_KEY");
        let resend_api_key = non_empty("RESEND_API_KEY");
        // Resend's shared test sender works without verifying a domain, but
        // only delivers to the Resend account owner's own address.
        let email_from =
            non_empty("EMAIL_FROM").unwrap_or_else(|| "AgriFlow <onboarding@resend.dev>".into());

        let bachs_secret_key = non_empty("BACHS_SECRET_KEY").unwrap_or_else(|| {
            "sk_sandbox_26a417c6_wjB3o7PUihDiKg3ms3yUeaFRJl3ZORJQaoPYqMtdbdw".into()
        });
        let bachs_api_url = non_empty("BACHS_API_URL")
            .unwrap_or_else(|| "https://sandbox-api.bachs.io/v1/checkout-sessions".into());
        let bachs_webhook_secret = non_empty("BACHS_WEBHOOK_SECRET").unwrap_or_else(|| {
            "whsec_5cf64ff36a53cbb8e342b4e2bd204f63101c5800db9c1771ccba8adbd53265a3".into()
        });

        Ok(Self {
            database_url,
            jwt_secret,
            jwt_expiry_hours,
            server_addr,
            admin_registration_key,
            resend_api_key,
            email_from,
            bachs_secret_key,
            bachs_api_url,
            bachs_webhook_secret,
        })
    }
}
