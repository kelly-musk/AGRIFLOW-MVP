//! Server-side Bachs.io integration: creating checkout sessions and
//! verifying incoming webhooks.
//!
//! This replaces a frontend implementation that called Bachs directly from
//! the browser with a hardcoded secret key -- moving it here means that key
//! (and the webhook signing secret) never has to touch client code again,
//! regardless of whether they're shared/reusable sandbox credentials or not.
//! See API_AUDIT.md and the payments punch list for the history.
//!
//! Signature scheme per https://docs.bachs.io/guides/webhooks/overview:
//! HMAC-SHA256 over `"{timestamp}.{raw_body}"`, hex-encoded, carried in the
//! `X-Bachs-Signature-V2` header alongside `X-Bachs-Timestamp` (Unix
//! seconds, 300s replay tolerance). The raw body must be verified before
//! any JSON parsing -- re-serializing first can change whitespace/byte
//! order and silently break verification.

use hmac::{Hmac, KeyInit, Mac};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;

type HmacSha256 = Hmac<Sha256>;

const REPLAY_TOLERANCE_SECONDS: i64 = 300;

#[derive(Clone)]
pub struct BachsClient {
    client: reqwest::Client,
    secret_key: String,
    api_url: String,
    webhook_secret: String,
}

#[derive(Debug, Serialize)]
struct CreateCheckoutSessionRequest<'a> {
    pricing: Pricing<'a>,
    customer: Customer<'a>,
    success_url: String,
    cancel_url: String,
    metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
struct Pricing<'a> {
    amount: String,
    currency: &'a str,
}

#[derive(Debug, Serialize)]
struct Customer<'a> {
    email: &'a str,
    name: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckoutSession {
    pub checkout_id: String,
    pub checkout_url: String,
}

/// The subset of a Bachs webhook event this app acts on. Extra fields are
/// ignored by serde's default behavior.
#[derive(Debug, Deserialize)]
pub struct WebhookEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: WebhookEventData,
}

#[derive(Debug, Deserialize)]
pub struct WebhookEventData {
    pub checkout_id: Option<String>,
}

impl BachsClient {
    pub fn new(secret_key: String, api_url: String, webhook_secret: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            secret_key,
            api_url,
            webhook_secret,
        }
    }

    pub async fn create_checkout_session(
        &self,
        amount: Decimal,
        currency: &str,
        customer_email: &str,
        customer_name: &str,
        success_url: String,
        cancel_url: String,
        transaction_id: &str,
    ) -> anyhow::Result<CheckoutSession> {
        let mut metadata = HashMap::new();
        metadata.insert("transaction_id".to_string(), transaction_id.to_string());
        metadata.insert("platform".to_string(), "AgriFlow".to_string());
        metadata.insert("escrow".to_string(), "true".to_string());

        let body = CreateCheckoutSessionRequest {
            pricing: Pricing {
                amount: format!("{amount:.2}"),
                currency,
            },
            customer: Customer {
                email: customer_email,
                name: customer_name,
            },
            success_url,
            cancel_url,
            metadata,
        };

        let res = self
            .client
            .post(&self.api_url)
            .bearer_auth(&self.secret_key)
            .json(&body)
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("Bachs API returned {status}: {text}");
        }

        Ok(res.json::<CheckoutSession>().await?)
    }

    /// Verifies a webhook delivery's signature and freshness. `raw_body`
    /// must be the exact, unparsed request bytes.
    pub fn verify_webhook(&self, raw_body: &[u8], signature_hex: &str, timestamp: &str) -> bool {
        let Ok(ts) = timestamp.parse::<i64>() else {
            return false;
        };
        let now = chrono::Utc::now().timestamp();
        if (now - ts).abs() > REPLAY_TOLERANCE_SECONDS {
            return false;
        }

        let Ok(sig_bytes) = hex::decode(signature_hex) else {
            return false;
        };
        let Ok(mut mac) = HmacSha256::new_from_slice(self.webhook_secret.as_bytes()) else {
            return false;
        };
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(raw_body);
        mac.verify_slice(&sig_bytes).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> BachsClient {
        BachsClient::new("sk_test".into(), "https://example.invalid".into(), "whsec_test".into())
    }

    fn sign(secret: &str, timestamp: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn accepts_a_correctly_signed_fresh_delivery() {
        let bachs = client();
        let body = br#"{"type":"collection.succeeded"}"#;
        let ts = chrono::Utc::now().timestamp().to_string();
        let sig = sign("whsec_test", &ts, body);
        assert!(bachs.verify_webhook(body, &sig, &ts));
    }

    #[test]
    fn rejects_a_signature_from_the_wrong_secret() {
        let bachs = client();
        let body = br#"{"type":"collection.succeeded"}"#;
        let ts = chrono::Utc::now().timestamp().to_string();
        let forged = sign("attacker-guessed-secret", &ts, body);
        assert!(!bachs.verify_webhook(body, &forged, &ts));
    }

    #[test]
    fn rejects_a_tampered_body_even_with_a_valid_signature_for_the_original() {
        let bachs = client();
        let original = br#"{"type":"collection.succeeded"}"#;
        let ts = chrono::Utc::now().timestamp().to_string();
        let sig = sign("whsec_test", &ts, original);
        let tampered = br#"{"type":"collection.succeeded","amount":"999999.00"}"#;
        assert!(!bachs.verify_webhook(tampered, &sig, &ts));
    }

    #[test]
    fn rejects_a_stale_timestamp_outside_the_replay_window() {
        let bachs = client();
        let body = br#"{"type":"collection.succeeded"}"#;
        let stale_ts = (chrono::Utc::now().timestamp() - 3600).to_string(); // 1h old
        let sig = sign("whsec_test", &stale_ts, body);
        assert!(!bachs.verify_webhook(body, &sig, &stale_ts));
    }

    #[test]
    fn rejects_malformed_signature_or_timestamp_without_panicking() {
        let bachs = client();
        let body = b"{}";
        assert!(!bachs.verify_webhook(body, "not-hex!!", "not-a-number"));
        assert!(!bachs.verify_webhook(body, "", ""));
    }
}
