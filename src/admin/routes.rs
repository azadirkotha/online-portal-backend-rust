use axum::{
    extract::DefaultBodyLimit,
    http::{header, HeaderValue},
    middleware::from_fn_with_state,
    routing::{get, post},
    Router,
};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};
use super::brand;
use super::handlers::{header_highlights, account, articles, auth, categories, dashboard, front_page, menus, roles, settings, sub_categories, tags, uploads, journalists, users, breaking_news};
use super::middleware::require_admin;
use crate::state::AppState;
use crate::cache::purge_on_admin_write;

// Scripts only from this origin. 'unsafe-inline' is allowed for STYLES only, because the rich-text
// editor and the saved article HTML use inline style="..." attributes (fonts, sizes, colours).
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; frame-src 'self' blob:; \
object-src 'none'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'";

pub fn create_routes(state: AppState) -> Router {
    let upload_dir = state.config.upload_dir.clone();

    // site name + logo from the settings, for the sidebar and the sign-in page
    brand::start(state.db_pool.clone());

    let protected = Router::new()
        .route("/", get(dashboard::index))
        .route("/logout", post(auth::logout))
        // users
        .route("/users", get(users::list).post(users::create))
        .route("/users/new", get(users::new_form))
        .route("/users/:id/edit", get(users::edit_form))
        .route("/users/:id", post(users::update))
        // roles
        .route("/roles", get(roles::list).post(roles::create))
        .route("/roles/new", get(roles::new_form))
        .route("/roles/:id/edit", get(roles::edit_form))
        .route("/roles/:id", post(roles::update))
        .route("/roles/:id/delete", post(roles::delete))
        // categories
        .route("/categories", get(categories::list).post(categories::create))
        .route("/categories/new", get(categories::new_form))
        .route("/categories/:id/edit", get(categories::edit_form))
        .route("/categories/:id", post(categories::update))
        .route("/categories/:id/delete", post(categories::delete))
        // sub categories
        .route("/sub-categories", get(sub_categories::list).post(sub_categories::create))
        .route("/sub-categories/new", get(sub_categories::new_form))
        .route("/sub-categories/:id/edit", get(sub_categories::edit_form))
        .route("/sub-categories/:id", post(sub_categories::update))
        .route("/sub-categories/:id/delete", post(sub_categories::delete))
        // tags
        .route("/tags", get(tags::list).post(tags::create))
        .route("/tags/new", get(tags::new_form))
        .route("/tags/:id/edit", get(tags::edit_form))
        .route("/tags/:id", post(tags::update))
        .route("/tags/:id/delete", post(tags::delete))
        // articles + review workflow
        .route("/articles", get(articles::list).post(articles::create))
        .route("/articles/new", get(articles::new_form))
        .route("/articles/:id", get(articles::show).post(articles::update))
        .route("/articles/:id/edit", get(articles::edit_form))
        .route("/articles/:id/submit", post(articles::submit))
        .route("/articles/:id/publish", post(articles::publish))
        .route("/articles/:id/request-changes", post(articles::request_changes))
        .route("/articles/:id/unpublish", post(articles::unpublish))
        .route("/articles/:id/delete", post(articles::delete))
        // image uploads for the editor (6 MB request limit; the handler enforces 5 MB per image)
        .route("/media/upload", post(uploads::upload_image).layer(DefaultBodyLimit::max(6 * 1024 * 1024)))
        // front page
        .route("/front-page", get(front_page::main_page).post(front_page::save_main))
        .route("/front-page/categories", get(front_page::categories_page).post(front_page::save_categories))
        .route("/front-page/leads", get(front_page::leads_page).post(front_page::save_leads))
        .route("/front-page/header", get(header_highlights::show).post(header_highlights::save))
        // site menus
        .route("/menus", get(menus::index))
        .route("/menus/items/:id/edit", get(menus::edit_form))
        .route("/menus/items/:id", post(menus::update))
        .route("/menus/items/:id/delete", post(menus::delete))
        .route("/menus/:location", get(menus::builder))
        .route("/menus/:location/order", post(menus::save_order))
        .route("/menus/:location/add-categories", post(menus::add_categories))
        .route("/menus/:location/add-sub-categories", post(menus::add_sub_categories))
        .route("/menus/:location/add-link", post(menus::add_link))
        // site settings
        .route("/settings", get(settings::show).post(settings::save))
        // my account (every signed-in dashboard user)
        .route("/account", get(account::show).post(account::update))
        .route("/account/password", post(account::change_password))
        .route("/users/:id/delete", post(users::delete))
        .route("/journalists", get(journalists::list))
        .route("/journalists/:id", get(journalists::detail))
        .route("/journalists/:id/file/:kind", get(journalists::file))
        .route("/journalists/:id/approve", post(journalists::approve))
        .route("/journalists/:id/reject", post(journalists::reject))
        .route("/journalists/new", get(journalists::new_form).post(journalists::create))
        .route("/journalists/:id/edit", get(journalists::edit_form).post(journalists::update))
        .route("/breaking-news", get(breaking_news::list).post(breaking_news::create))
        .route("/breaking-news/new", get(breaking_news::new_form))
        .route("/breaking-news/bulk", post(breaking_news::bulk))
        .route("/breaking-news/:id/edit", get(breaking_news::edit_form))
        .route("/breaking-news/:id", post(breaking_news::update))
        .route("/breaking-news/:id/delete", post(breaking_news::delete))
        .route_layer(from_fn_with_state(state.clone(), purge_on_admin_write))
        .route_layer(from_fn_with_state(state.clone(), require_admin));

    // NOTE: no CORS layer on purpose. The admin is same-origin only.
    Router::new()
        .route("/login", get(auth::login_page).post(auth::login_submit))
        .merge(protected)
        .nest_service("/static", ServeDir::new("static/admin"))
        .nest_service("/uploads", ServeDir::new(upload_dir)) // so images show up in the editor / preview
        .layer(SetResponseHeaderLayer::overriding(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP)))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
        .with_state(state)
}