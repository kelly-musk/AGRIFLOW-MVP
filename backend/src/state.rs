use sqlx::PgPool;

use crate::bachs::BachsClient;
use crate::config::Config;
use crate::email::Mailer;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Config,
    pub mailer: Mailer,
    pub bachs: BachsClient,
}
