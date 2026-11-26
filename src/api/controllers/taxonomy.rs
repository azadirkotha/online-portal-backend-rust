use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::models::{category, tag};
use crate::state::AppState;

fn server_error(e: sqlx::Error) -> Response {
    eprintln!("taxonomy api error: {e:?}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Something went wrong"}))).into_response()
}

fn not_found(what: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": format!("{what} not found")}))).into_response()
}

/// GET /api/categories : active categories, each with its active sub categories
pub async fn list_categories(State(state): State<AppState>) -> Response {
    match category::public_tree(&state.db_pool).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => server_error(e),
    }
}

/// GET /api/categories/:slug
pub async fn get_category(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    match category::public_by_slug(&state.db_pool, &slug).await {
        Ok(Some(c)) => Json(c).into_response(),
        Ok(None) => not_found("Category"),
        Err(e) => server_error(e),
    }
}

/// GET /api/tags
pub async fn list_tags(State(state): State<AppState>) -> Response {
    match tag::list(&state.db_pool).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => server_error(e),
    }
}

/// GET /api/tags/:slug
pub async fn get_tag(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    match tag::find_by_slug(&state.db_pool, &slug).await {
        Ok(Some(t)) => Json(t).into_response(),
        Ok(None) => not_found("Tag"),
        Err(e) => server_error(e),
    }
}
