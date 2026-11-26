//! Public comment endpoints.
//! Save as src/api/controllers/comments.rs and add `pub mod comments;` to controllers/mod.rs.
//!
//!   GET  /api/articles/:slug/comments?page=1
//!   POST /api/articles/:slug/comments   { name, body, parent_id?, website? }

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::models::public_comments::{self as comments, AddError, CommentPage, MAX_BODY, MAX_NAME};
use crate::state::AppState;

fn fail(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn server_error(e: sqlx::Error) -> Response {
    eprintln!("comments api error: {e:?}");
    fail(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong")
}

/// Drop control characters; newlines survive only in the comment body.
fn clean(s: &str, keep_newlines: bool) -> String {
    s.chars()
        .filter(|c| !c.is_control() || (keep_newlines && *c == '\n'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// First address in X-Forwarded-For (set by your reverse proxy), else X-Real-IP.
fn client_ip(h: &HeaderMap) -> Option<String> {
    let first = |name: &str| {
        h.get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    first("x-forwarded-for").or_else(|| first("x-real-ip"))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
}

async fn load_comments(
    pool: sqlx::MySqlPool,
    slug: String,
    page: i64,
) -> Result<Option<CommentPage>, sqlx::Error> {
    let Some(id) = comments::article_id(&pool, &slug).await? else {
        return Ok(None);
    };
    comments::list(&pool, id, page).await.map(Some)
}

/// GET /api/articles/:slug/comments?page=1 (cached; a new comment clears the article's entries)
pub async fn list(State(state): State<AppState>, Path(slug): Path<String>, Query(q): Query<ListQuery>) -> Response {
    let page = q.page.unwrap_or(1).clamp(1, 1000);
    let key = format!("comments|{slug}|{page}");
    let load = load_comments(state.db_pool.clone(), slug, page);

    match state.cache.json_opt(key, load).await {
        Ok(Some(res)) => res,
        Ok(None) => fail(StatusCode::NOT_FOUND, "Article not found"),
        Err(e) => server_error(e),
    }
}

#[derive(Deserialize)]
pub struct NewComment {
    pub name: String,
    pub body: String,
    /// id of the comment (or reply) being answered
    pub parent_id: Option<i32>,
    /// honeypot: real visitors never fill this in
    #[serde(default)]
    pub website: String,
}

/// POST /api/articles/:slug/comments
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<NewComment>,
) -> Response {
    // bots fill the hidden field: pretend it worked and store nothing
    if !input.website.trim().is_empty() {
        return (StatusCode::CREATED, Json(json!({ "status": "pending" }))).into_response();
    }

    let name = clean(&input.name, false);
    let body = clean(&input.body, true);
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, "Invalid name");
    }
    if body.is_empty() || body.chars().count() > MAX_BODY {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, "Invalid comment");
    }

    // one comment per visitor every few seconds (skipped when no proxy header is present)
    if let Some(ip) = client_ip(&headers) {
        if !state.cache.comment_allowed(&ip).await {
            return fail(StatusCode::TOO_MANY_REQUESTS, "Too many comments, try again shortly");
        }
    }

    let article_id = match comments::article_id(&state.db_pool, &slug).await {
        Ok(Some(id)) => id,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "Article not found"),
        Err(e) => return server_error(e),
    };

    match comments::add(&state.db_pool, article_id, input.parent_id, &name, &body).await {
        Ok(item) => {
            state.cache.forget_comments(&slug);
            (StatusCode::CREATED, Json(item)).into_response()
        }
        Err(AddError::NoParent) => fail(StatusCode::BAD_REQUEST, "The comment you replied to was not found"),
        Err(AddError::Db(e)) => server_error(e),
    }
}