use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::models::site_settings;
use crate::state::AppState;

/// GET /api/settings : portal name, slogan, logo, publisher, editor, address, contacts, social links...
/// Every key is always present (empty string when not set); image values are absolute URLs.
pub async fn show(State(state): State<AppState>) -> Response {
    match site_settings::public(&state.db_pool, &state.config.public_base_url).await {
        Ok(map) => Json(map).into_response(),
        Err(e) => {
            eprintln!("settings api error: {e:?}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Something went wrong"}))).into_response()
        }
    }
}
