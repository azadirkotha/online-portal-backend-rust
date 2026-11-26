use sqlx::MySqlPool;

use crate::cache::PublicCache;
use crate::config::AppConfig;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: MySqlPool,
    pub config: AppConfig,
    pub cache: PublicCache,
}