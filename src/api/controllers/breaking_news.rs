use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::models::breaking_news;
use crate::state::AppState;

/// Default and maximum number of headlines returned.
const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 50;

fn server_error(e: sqlx::Error) -> Response {
    eprintln!("breaking news api error: {e:?}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Something went wrong"}))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct BreakingQuery {
    pub limit: Option<i64>,
}

/// GET /api/breaking-news?limit=20 : the checked (active) headlines, newest first
pub async fn list(State(state): State<AppState>, Query(q): Query<BreakingQuery>) -> Response {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let load = breaking_news::public_list(&state.db_pool, limit);
    match state.cache.json(format!("breaking:{limit}"), load).await {
        Ok(res) => res,
        Err(e) => server_error(e),
    }
}