use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::json;

use crate::models::portal::{self, Urls};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct HeaderQuery {
    /// YYYY-MM-DD, for the past-day portal
    pub date: Option<String>,
}

fn urls(state: &AppState) -> Urls<'_> {
    Urls {
        category: &state.config.menu_category_url,
        sub_category: &state.config.menu_sub_category_url,
        article: &state.config.article_url,
    }
}

fn server_error(e: sqlx::Error) -> Response {
    eprintln!("portal api error: {e:?}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Something went wrong"}))).into_response()
}

/// GET /api/header[?date=YYYY-MM-DD] : site info, main menu, categories (for the search panel), featured stories
pub async fn header(State(state): State<AppState>, Query(q): Query<HeaderQuery>) -> Response {
    let date = match q.date.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        None => None,
        Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
            Ok(date) => Some(date),
            Err(_) => {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": "date must look like 2026-10-31"}))).into_response()
            }
        },
    };
    let key = match date {
        Some(d) => format!("header:{d}"),
        None => "header".to_string(),
    };
    let urls = urls(&state);
    let load = portal::header(&state.db_pool, &state.config.public_base_url, &urls, date);
    match state.cache.json(key, load).await {
        Ok(res) => res,
        Err(e) => server_error(e),
    }
}

/// GET /api/footer : contact details, social links and the three footer menus
pub async fn footer(State(state): State<AppState>) -> Response {
    let urls = urls(&state);
    let load = portal::footer(&state.db_pool, &state.config.public_base_url, &urls);
    match state.cache.json("footer".to_string(), load).await {
        Ok(res) => res,
        Err(e) => server_error(e),
    }
}