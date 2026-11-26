use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::models::menu::{self, UrlPatterns};
use crate::state::AppState;

fn server_error(e: sqlx::Error) -> Response {
    eprintln!("menus api error: {e:?}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Something went wrong"}))).into_response()
}

/// GET /api/menus : every menu, keyed by location (header, footer_quick, footer_sections, footer_about)
pub async fn all(State(state): State<AppState>) -> Response {
    let patterns = UrlPatterns {
        category: &state.config.menu_category_url,
        sub_category: &state.config.menu_sub_category_url,
    };
    match menu::public_menus(&state.db_pool, &patterns).await {
        Ok(m) => Json(m).into_response(),
        Err(e) => server_error(e),
    }
}

/// GET /api/menus/:location
pub async fn one(State(state): State<AppState>, Path(location): Path<String>) -> Response {
    if menu::location_label(&location).is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "Menu not found"}))).into_response();
    }
    let patterns = UrlPatterns {
        category: &state.config.menu_category_url,
        sub_category: &state.config.menu_sub_category_url,
    };
    match menu::public_menus(&state.db_pool, &patterns).await {
        Ok(mut m) => Json(m.remove(&location).unwrap_or_default()).into_response(),
        Err(e) => server_error(e),
    }
}
