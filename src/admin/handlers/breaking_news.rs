use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::extract::Form;
use serde::Deserialize;
use tower_sessions::Session;

use super::auth::CsrfOnly;
use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::Layout;
use crate::models::breaking_news::{self as bn, BreakingInput, BreakingRow, PER_PAGE};
use crate::state::AppState;

fn can_manage(a: &CurrentAdmin) -> bool {
    a.has("breaking:manage")
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A site path ("/news/slug") or an http(s) URL, no spaces.
fn valid_path(p: &str) -> bool {
    p.len() <= 500
        && !p.chars().any(|c| c.is_whitespace() || c.is_control())
        && ((p.starts_with('/') && !p.starts_with("//")) || p.starts_with("https://") || p.starts_with("http://"))
}

// ---------- templates ----------

#[derive(Template)]
#[template(path = "admin/breaking/list.html")]
struct ListTemplate {
    layout: Layout,
    rows: Vec<BreakingRow>,
    q: String,
    q_enc: String,
    page: i64,
    pages: i64,
    total: i64,
    active_count: i64,
    has_prev: bool,
    has_next: bool,
    shown_from: i64,
    shown_to: i64,
}

#[derive(Template)]
#[template(path = "admin/breaking/form.html")]
struct FormTemplate {
    layout: Layout,
    id: i32,
    is_edit: bool,
    values: BreakingInput,
}

// ---------- list ----------

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    page: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Query(q): Query<ListQuery>,
) -> Response {
    if !can_manage(&admin) {
        return forbidden();
    }
    let q_text: String = q.q.as_deref().unwrap_or("").trim().chars().take(100).collect();
    let page = q.page.unwrap_or(1).max(1);

    let (rows, total) = match bn::list(&state.db_pool, &q_text, page).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);
    if page > pages {
        return Redirect::to(&format!("/breaking-news?q={}&page={}", enc(&q_text), pages)).into_response();
    }
    let active_count = match bn::active_count(&state.db_pool).await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let layout = build_layout(&session, &admin).await;
    render(ListTemplate {
        layout,
        rows,
        q_enc: enc(&q_text),
        q: q_text,
        page,
        pages,
        total,
        active_count,
        has_prev: page > 1,
        has_next: page < pages,
        shown_from: if total == 0 { 0 } else { (page - 1) * PER_PAGE + 1 },
        shown_to: (page * PER_PAGE).min(total),
    })
}

// ---------- create / edit ----------

#[derive(Deserialize)]
pub struct BreakingForm {
    csrf_token: String,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    path: String,
    /// a checkbox: present when ticked
    #[serde(default)]
    is_active: Option<String>,
}

impl BreakingForm {
    fn parse(&self) -> (BreakingInput, Option<&'static str>) {
        let headline = self.headline.split_whitespace().collect::<Vec<_>>().join(" ");
        let path = self.path.trim().to_string();
        let error = if headline.is_empty() || headline.chars().count() > 255 {
            Some("A headline is required (max 255 characters).")
        } else if !valid_path(&path) {
            Some("The path must start with / (for example /news/my-story) or be a full http(s) link.")
        } else {
            None
        };
        (BreakingInput { headline, path, is_active: self.is_active.is_some() }, error)
    }
}

async fn form_page(
    session: &Session,
    admin: &CurrentAdmin,
    id: Option<i32>,
    values: BreakingInput,
    error: Option<String>,
) -> Response {
    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }
    render(FormTemplate { layout, id: id.unwrap_or(0), is_edit: id.is_some(), values })
}

pub async fn new_form(Extension(admin): Extension<CurrentAdmin>, session: Session) -> Response {
    if !can_manage(&admin) {
        return forbidden();
    }
    let values = BreakingInput { is_active: true, ..Default::default() };
    form_page(&session, &admin, None, values, None).await
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<BreakingForm>,
) -> Response {
    if !can_manage(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&session, &admin, None, input, Some(msg.into())).await;
    }
    match bn::create(&state.db_pool, admin.id, &input).await {
        Ok(_) => {
            flash(&session, "Breaking headline added.").await;
            Redirect::to("/breaking-news").into_response()
        }
        Err(e) => internal_error(e),
    }
}

pub async fn edit_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    if !can_manage(&admin) {
        return forbidden();
    }
    let row = match bn::find(&state.db_pool, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    let values = BreakingInput { headline: row.headline, path: row.path, is_active: row.is_active };
    form_page(&session, &admin, Some(id), values, None).await
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<BreakingForm>,
) -> Response {
    if !can_manage(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match bn::find(&state.db_pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    }
    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&session, &admin, Some(id), input, Some(msg.into())).await;
    }
    match bn::update(&state.db_pool, id, &input).await {
        Ok(()) => {
            flash(&session, "Saved.").await;
            Redirect::to("/breaking-news").into_response()
        }
        Err(e) => internal_error(e),
    }
}

// ---------- checkbox selection (bulk) ----------

#[derive(Deserialize)]
pub struct BulkForm {
    csrf_token: String,
    #[serde(default)]
    page_ids: Vec<i32>,
    #[serde(default)]
    active_ids: Vec<i32>,
    #[serde(default)]
    q: String,
    #[serde(default)]
    page: i64,
}

pub async fn bulk(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<BulkForm>,
) -> Response {
    if !can_manage(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    if let Err(e) = bn::set_active_on_page(&state.db_pool, &f.page_ids, &f.active_ids).await {
        return internal_error(e);
    }
    flash(&session, "Breaking list updated.").await;
    Redirect::to(&format!("/breaking-news?q={}&page={}", enc(f.q.trim()), f.page.max(1))).into_response()
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !can_manage(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match bn::delete(&state.db_pool, id).await {
        Ok(_) => {
            flash(&session, "Headline deleted.").await;
            Redirect::to("/breaking-news").into_response()
        }
        Err(e) => internal_error(e),
    }
}