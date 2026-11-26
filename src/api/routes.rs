use axum::{
    extract::DefaultBodyLimit,
    http::{header, HeaderValue},
    middleware::from_fn_with_state,
    routing::{get, post},
    Router,
};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

use crate::api::controllers;
use crate::api::controllers::portal;
use crate::cache::cached_get;
use crate::state::AppState;

pub fn create_routes(state: AppState) -> Router {
    let upload_dir = state.config.upload_dir.clone();

    // Read-only, identical-for-everyone endpoints: cached by path + query.
    let cached = Router::new()
        .route("/categories", get(controllers::taxonomy::list_categories))
        .route("/categories/:slug", get(controllers::taxonomy::get_category))
        .route("/tags", get(controllers::taxonomy::list_tags))
        .route("/tags/:slug", get(controllers::taxonomy::get_tag))
        .route("/menus", get(controllers::menus::all))
        .route("/menus/:location", get(controllers::menus::one))
        .route("/settings", get(controllers::settings::show))
        .route_layer(from_fn_with_state(state.clone(), cached_get));

    Router::new()
        .route("/share/:slug", get(controllers::articles::share))
        .nest(
            "/api",
            Router::new()
                .route("/auth/register", post(controllers::auth::register))
                .route("/auth/login", post(controllers::auth::login))
                .route("/auth/profile", get(controllers::auth::get_profile))
                // public, read-only (each has its own cache inside the handler)
                .route("/home", get(controllers::articles::home))
                .route("/articles/:slug", get(controllers::articles::detail))
                .route("/articles/:slug/comments", get(controllers::comments::list).post(controllers::comments::create).layer(DefaultBodyLimit::max(16 * 1024)),)
                .route("/categories/:slug/page", get(controllers::articles::category_page))
                .route("/tags/:slug/articles", get(controllers::articles::tag_articles))
                .route("/breaking-news", get(controllers::breaking_news::list))
                .route("/search", get(controllers::articles::search))
                .route("/search/categories", get(controllers::articles::search_categories))
                .route("/header", get(controllers::portal::header))
                .route("/footer", get(portal::footer))
                // public, read-only, cached by the cached_get layer
                .merge(cached)
                // journalists
                .route("/journalist/me", get(controllers::journalist::me))
                // public application form
                .route("/journalists/apply", post(controllers::journalist::apply).layer(DefaultBodyLimit::max(16 * 1024 * 1024)))
                // approved journalists (JWT)
                .route("/journalist/password", post(controllers::journalist::change_password))
                .route("/journalist/categories", get(controllers::journalist::categories))
                .route("/journalist/articles", get(controllers::journalist::list_articles).post(controllers::journalist::create_article))
                .route(
                    "/journalist/articles/:id",
                    get(controllers::journalist::get_article)
                        .put(controllers::journalist::update_article)
                        .delete(controllers::journalist::delete_article),
                )
                .route("/journalist/uploads", post(controllers::journalist::upload_image).layer(DefaultBodyLimit::max(6 * 1024 * 1024)))
                // admins (JWT + permission)
                .route("/admin/journalist-applications", get(controllers::journalist::admin_list))
                .route("/admin/journalist-applications/:id", get(controllers::journalist::admin_detail))
                .route("/admin/journalist-applications/:id/files/:kind", get(controllers::journalist::admin_file))
                .route("/admin/journalist-applications/:id/approve", post(controllers::journalist::admin_approve))
                .route("/admin/journalist-applications/:id/reject", post(controllers::journalist::admin_reject))
                .route("/journalists/verify", get(controllers::journalist::verify)),
        )
        // uploaded article images (read-only)
        .nest_service("/uploads", ServeDir::new(upload_dir))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .with_state(state)
}