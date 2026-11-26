use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;
use std::collections::HashSet;
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::Layout;
use crate::models::front_page::{self, Brief};
use crate::models::portal::{self, HEADER_HIGHLIGHTS};
use crate::state::AppState;

#[derive(Template)]
#[template(path = "admin/front_page/header.html")]
struct HeaderTemplate {
    layout: Layout,
    items: Vec<Brief>,
    pool: Vec<Brief>,
    capacity: usize,
}

pub async fn show(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("frontpage:manage") {
        return forbidden();
    }
    let items = match portal::selected(&state.db_pool).await {
        Ok(i) => i,
        Err(e) => return internal_error(e),
    };
    let pool = match portal::pool(&state.db_pool, 80).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };
    let layout = build_layout(&session, &admin).await;
    render(HeaderTemplate { layout, items, pool, capacity: HEADER_HIGHLIGHTS })
}

#[derive(Deserialize)]
pub struct HeaderForm {
    csrf_token: String,
    #[serde(default)]
    ids: String,
}

pub async fn save(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<HeaderForm>,
) -> Response {
    if !admin.has("frontpage:manage") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let requested: Vec<i32> = f.ids.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    let published = match front_page::published_among(&state.db_pool, &requested).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };

    // only published stories, each once, no more than the header holds
    let mut seen: HashSet<i32> = HashSet::new();
    let ids: Vec<i32> = requested
        .into_iter()
        .filter(|id| published.contains(id) && seen.insert(*id))
        .take(HEADER_HIGHLIGHTS)
        .collect();

    match portal::save(&state.db_pool, &ids).await {
        Ok(()) => {
            flash(&session, "Header highlights saved.").await;
            Redirect::to("/front-page/header").into_response()
        }
        Err(e) => internal_error(e),
    }
}
