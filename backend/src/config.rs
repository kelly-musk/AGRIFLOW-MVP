use std::net::SocketAddr;

use crate::storage::StorageConfig;

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
    /// Bachs.io API key (`sk_sandbox_...` or `sk_live_...`). Unset → Bachs
    /// checkout is disabled and the checkout-session endpoint returns 503.
    pub bachs_secret_key: Option<String>,
    /// Signing secret for the Bachs webhook endpoint (`whsec_...`). Unset →
    /// every webhook delivery is rejected, since none can be verified.
    pub bachs_webhook_secret: Option<String>,
    /// Public origin of the React app, e.g. `https://agri-flowmvp.vercel.app`
    /// (no trailing slash). Bachs checkout redirects are only allowed back to
    /// this origin, and it builds the default success/cancel URLs.
    pub frontend_base_url: Option<String>,
    /// S3-compatible bucket for listing media. `None` (no `S3_*` vars set) →
    /// the media endpoints return 503.
    pub storage: Option<StorageConfig>,
    /// Origins allowed to upload to the bucket from a browser:
    /// `FRONTEND_BASE_URL` plus any in `S3_CORS_ORIGINS` (comma-separated,
    /// e.g. `http://localhost:5173`). Applied to the bucket at boot.
    pub storage_cors_origins: Vec<String>,
    /// JSON-RPC HTTP endpoint for the chain AgriFlowEscrow is deployed on.
    /// Defaults to a public Sepolia endpoint -- read-only log polling, no
    /// API key or private key needed. See `chain.rs`.
    pub chain_rpc_url: String,
    /// AgriFlowEscrow's deployed address. Defaults to the real Sepolia
    /// deployment (see docs/SMART_CONTRACT_SPEC_AND_GITHUB_ISSUES.md).
    pub escrow_contract_address: String,
    /// How often the indexer polls for new blocks.
    pub chain_poll_interval_secs: u64,
    /// First block to scan from on a cold start (no `indexer_cursor` row
    /// yet). Defaults to the real AgriFlowEscrow deployment block on
    /// Sepolia (11775184, confirmed live against
    /// ethereum-sepolia-rpc.publicnode.com's `OwnershipTransferred` log at
    /// construction) -- not block 0, and not found by probing `eth_getCode`
    /// at historical heights, since public RPC nodes prune old state
    /// (verified: that approach fails with `"state at block #... is
    /// pruned"`). `eth_getLogs`, which is all the indexer otherwise uses,
    /// doesn't have this problem.
    pub chain_start_block: u64,
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
        let bachs_secret_key = non_empty("BACHS_SECRET_KEY");
        let bachs_webhook_secret = non_empty("BACHS_WEBHOOK_SECRET");
        let frontend_base_url =
            non_empty("FRONTEND_BASE_URL").map(|v| v.trim().trim_end_matches('/').to_string());

        // All-or-nothing, so a half-configured bucket fails at boot instead
        // of on the first upload.
        let s3_vars = ["S3_ENDPOINT", "S3_BUCKET", "S3_ACCESS_KEY_ID", "S3_SECRET_ACCESS_KEY"];
        let storage = match s3_vars.map(non_empty) {
            [Some(endpoint), Some(bucket), Some(access_key_id), Some(secret_access_key)] => Some(StorageConfig {
                endpoint,
                bucket,
                // Railway Buckets and R2 accept "auto"; AWS needs the real region.
                region: non_empty("S3_REGION").unwrap_or_else(|| "auto".into()),
                access_key_id,
                secret_access_key,
                path_style: non_empty("S3_PATH_STYLE").is_some_and(|v| v == "true" || v == "1"),
            }),
            [None, None, None, None] => None,
            _ => anyhow::bail!("set all of {} or none of them", s3_vars.join(", ")),
        };

        let storage_cors_origins: Vec<String> = frontend_base_url
            .iter()
            .cloned()
            .chain(
                non_empty("S3_CORS_ORIGINS")
                    .unwrap_or_default()
                    .split(',')
                    .map(|o| o.trim().trim_end_matches('/').to_string())
                    .filter(|o| !o.is_empty()),
            )
            .collect();

        let chain_rpc_url = non_empty("CHAIN_RPC_URL")
            .unwrap_or_else(|| "https://ethereum-sepolia-rpc.publicnode.com".into());
        let escrow_contract_address = non_empty("ESCROW_CONTRACT_ADDRESS")
            .unwrap_or_else(|| "0x9E93B3ffF884b736fECEACa33d93f33aAfDdc6C5".into());
        let chain_poll_interval_secs = non_empty("CHAIN_POLL_INTERVAL_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(15);
        let chain_start_block = non_empty("CHAIN_START_BLOCK")
            .and_then(|v| v.parse().ok())
            .unwrap_or(11_775_184);

        Ok(Self {
            database_url,
            jwt_secret,
            jwt_expiry_hours,
            server_addr,
            admin_registration_key,
            resend_api_key,
            email_from,
            bachs_secret_key,
            bachs_webhook_secret,
            frontend_base_url,
            storage,
            storage_cors_origins,
            chain_rpc_url,
            escrow_contract_address,
            chain_poll_interval_secs,
            chain_start_block,
        })
    }
}
