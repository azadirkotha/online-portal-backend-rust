mod admin;
mod api;
mod config;
mod models;
mod security;
mod seed;
mod state;
mod cache;

use axum::http::{header::{AUTHORIZATION, CONTENT_TYPE}, HeaderValue, Method};
use sqlx::MySqlPool;
use tower_http::cors::CorsLayer;
use tower_sessions::{cookie::{time::Duration, SameSite}, Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::MySqlStore;

use crate::config::AppConfig;
use crate::state::AppState;
use crate::cache::PublicCache;

#[tokio::main]
async fn main() {
    let config = AppConfig::load();

    println!("Connecting to MariaDB...");
    let pool = MySqlPool::connect(&config.database_url)
        .await
        .expect("Failed to connect to MariaDB. Check that WAMP is running and DATABASE_URL is right.");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Database migrations failed");

    seed::run(&pool, &config).await.expect("Seeding failed");

    let state = AppState {
        db_pool: pool.clone(),
        config: config.clone(),
        cache: PublicCache::new(),
    };

        // Flush buffered article views to the database every 30 seconds
    {
        let st = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tick.tick().await;
                let counts = st.cache.take_views();
                if !counts.is_empty() {
                    models::public_news::add_views(&st.db_pool, &counts).await;
                }
            }
        });
    }

    // ---- Admin sessions (server-side, stored in MariaDB) ----
    let store = MySqlStore::new(pool.clone());
    store.migrate().await.expect("Failed to create the sessions table");
    let session_layer = SessionManagerLayer::new(store)
        .with_name("admin_sid")
        .with_secure(config.cookie_secure)
        .with_http_only(true)
        .with_same_site(SameSite::Strict)
        .with_expiry(Expiry::OnInactivity(Duration::minutes(30)));

    // ---- Public API: CORS limited to the Vue frontend's origin ----
    let origin: HeaderValue = config
        .frontend_origin
        .parse()
        .expect("FRONTEND_ORIGIN is not a valid origin");
    let cors = CorsLayer::new()
        .allow_origin(origin)
        // PUT and DELETE are needed by the journalist dashboard (edit / delete own articles)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([CONTENT_TYPE, AUTHORIZATION]);

    let api_app = api::routes::create_routes(state.clone()).layer(cors);
    // ---- Admin dashboard: no CORS, session cookie, own listener ----
    let admin_app = admin::routes::create_routes(state.clone()).layer(session_layer);

    let api_listener = tokio::net::TcpListener::bind(&config.server_port).await.expect("bind API port");
    let admin_listener = tokio::net::TcpListener::bind(&config.admin_bind).await.expect("bind admin port");

    println!("Public API   -> http://{}", config.server_port);
    println!("Admin panel  -> http://{}", config.admin_bind);

    let api_server = async move { axum::serve(api_listener, api_app).await };
    let admin_server = async move { axum::serve(admin_listener, admin_app).await };
    tokio::try_join!(api_server, admin_server).unwrap();
}