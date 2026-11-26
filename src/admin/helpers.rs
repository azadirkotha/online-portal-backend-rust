use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use tower_sessions::Session;

use super::csrf;
use super::middleware::CurrentAdmin;
use super::views::Layout;

pub fn render<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => internal_error(e),
    }
}

pub fn internal_error(e: impl std::fmt::Debug) -> Response {
    eprintln!("[admin] internal error: {e:?}");
    (StatusCode::INTERNAL_SERVER_ERROR, Html("<h1>500 - Something went wrong</h1>")).into_response()
}

pub fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, Html("<h1>403 - Forbidden</h1>")).into_response()
}

pub fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Html("<h1>404 - Not found</h1>")).into_response()
}

/// One-shot message shown on the next rendered page.
pub async fn flash(session: &Session, msg: &str) {
    let _ = session.insert("flash", msg.to_string()).await;
}

pub async fn build_layout(session: &Session, admin: &CurrentAdmin) -> Layout {
    let flash = session.remove::<String>("flash").await.ok().flatten();
    let display_name = if admin.name.trim().is_empty() { admin.username.clone() } else { admin.name.clone() };
    let initial = display_name.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    Layout {
        username: admin.username.clone(),
        display_name,
        photo: admin.photo.clone(),
        initial,
        csrf: csrf::token(session).await,
        can_users: admin.has("user:read"),
        can_journalists: admin.has("journalist:review"),
        can_roles: admin.has("role:read"),
        can_categories: admin.has("category:read"),
        can_sub_categories: admin.has("subcategory:read"),
        can_tags: admin.has("tag:read"),
        can_articles: admin.has("article:write") || admin.has("article:review"),
        can_front_page: admin.has("frontpage:manage"),
        can_menus: admin.has("menu:read"),
        can_settings: admin.has("settings:read"),
        can_breaking: admin.has("breaking:manage"),
        flash,
    }
}

pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false)
}