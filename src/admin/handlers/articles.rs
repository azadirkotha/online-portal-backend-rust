use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::extract::Form;
use serde::Deserialize;
use sqlx::MySqlPool;
use std::collections::HashMap;
use tower_sessions::Session;

use super::auth::CsrfOnly;
use super::common::is_fk_violation;
use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::{CheckOption, Layout};
use crate::models::article::{
    self, ArticleFull, ArticleInput, ArticleRow, ListFilter, Transition, KINDS, PAGE_SIZES,
};
use crate::models::{category, sanitize, tag};
use crate::state::AppState;

// ---------- who may do what ----------

fn is_reviewer(a: &CurrentAdmin) -> bool {
    a.has("article:review")
}
fn can_write(a: &CurrentAdmin) -> bool {
    a.has("article:write") || a.has("article:review")
}
fn can_view(a: &CurrentAdmin, author_id: i32) -> bool {
    is_reviewer(a) || (can_write(a) && author_id == a.id)
}
/// Reviewers can edit anything. Journalists only their own work that has not been submitted / published.
fn can_edit(a: &CurrentAdmin, author_id: i32, status: &str) -> bool {
    is_reviewer(a) || (a.has("article:write") && author_id == a.id && matches!(status, "draft" | "changes_requested"))
}
fn can_delete(a: &CurrentAdmin, author_id: i32, status: &str) -> bool {
    a.has("article:delete")
        || (a.has("article:write") && author_id == a.id && matches!(status, "draft" | "changes_requested"))
}

// ---------- templates ----------

struct StatusTab {
    key: &'static str,
    label: &'static str,
    count: i64,
    active: bool,
}

struct SizeOption {
    value: i64,
    active: bool,
}

/// One entry of the page-number bar: a page, the current page, or a "…" gap.
struct PageLink {
    page: i64,
    label: String,
    current: bool,
    gap: bool,
}

/// 1 … 4 5 [6] 7 8 … 20
fn page_links(page: i64, pages: i64) -> Vec<PageLink> {
    let mut out = Vec::new();
    let mut last = 0;
    for p in 1..=pages {
        let show = p == 1 || p == pages || (p - page).abs() <= 2;
        if !show {
            continue;
        }
        if p - last > 1 {
            out.push(PageLink { page: 0, label: "…".into(), current: false, gap: true });
        }
        out.push(PageLink { page: p, label: p.to_string(), current: p == page, gap: false });
        last = p;
    }
    out
}

struct AuthorOption {
    id: i32,
    name: String,
    selected: bool,
}

/// A YYYY-MM-DD date, or "" when missing / invalid.
fn valid_date(s: Option<&str>) -> String {
    s.map(str::trim)
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Percent-encodes text for a query string.
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

#[derive(Template)]
#[template(path = "admin/articles/list.html")]
struct ListTemplate {
    layout: Layout,
    rows: Vec<ArticleRow>,
    tabs: Vec<StatusTab>,
    status: String,
    // filters
    q: String,
    kind_opts: Vec<KindOption>,
    authors: Vec<AuthorOption>,
    date_from: String,
    date_to: String,
    filtered: bool,
    /// the active filters as `&q=..&kind=..` for the pager / tab links
    extra: String,
    is_reviewer: bool,
    page: i64,
    pages: i64,
    total: i64,
    per_page: i64,
    sizes: Vec<SizeOption>,
    links: Vec<PageLink>,
    shown_from: i64,
    shown_to: i64,
}

struct SubGroup {
    category_id: i32,
    category_name: String,
    items: Vec<CheckOption>,
}

struct KindOption {
    value: &'static str,
    label: &'static str,
    selected: bool,
}

#[derive(Template)]
#[template(path = "admin/articles/form.html")]
struct FormTemplate {
    layout: Layout,
    id: i32,
    is_edit: bool,
    status: String,
    review_note: String,
    values: ArticleInput,
    categories: Vec<CheckOption>,
    sub_groups: Vec<SubGroup>,
    tags: Vec<CheckOption>,
    kinds: Vec<KindOption>,
    is_reviewer: bool,
    can_submit: bool,
}

#[derive(Template)]
#[template(path = "admin/articles/show.html")]
struct ShowTemplate {
    layout: Layout,
    a: ArticleFull,
    status_label: &'static str,
    categories: Vec<String>,
    sub_categories: Vec<String>,
    tags: Vec<String>,
    can_edit: bool,
    can_submit: bool,
    can_publish: bool,
    can_request_changes: bool,
    can_unpublish: bool,
    can_delete: bool,
}

// ---------- form ----------

#[derive(Deserialize)]
pub struct ArticleForm {
    csrf_token: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    featured_image: String,
    #[serde(default)]
    featured_image_caption: String,
    #[serde(default)]
    news_source: String,
    #[serde(default)]
    category_ids: Vec<i32>,
    #[serde(default)]
    sub_category_ids: Vec<i32>,
    #[serde(default)]
    tag_ids: Vec<i32>,
    #[serde(default)]
    action: String, // save | submit | publish
}

impl ArticleForm {
    fn parse(&self) -> (ArticleInput, Option<&'static str>) {
        let title = self.title.trim().to_string();
        let body = sanitize::clean_html(&self.body);

        let mut summary = self.summary.trim().to_string();
        if summary.is_empty() {
            let text = sanitize::plain_text(&sanitize::without_figures(&body));
            summary = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(220).collect();
        }

        // Only images that were uploaded through this dashboard are accepted.
        let featured_image = self.featured_image.trim().to_string();
        let caption = self.featured_image_caption.trim().to_string();

        // single line, runs of whitespace collapsed
        let news_source = self.news_source.split_whitespace().collect::<Vec<_>>().join(" ");

        let error = if title.is_empty() || title.chars().count() > 255 {
            Some("A title is required (max 255 characters).")
        } else if news_source.is_empty() || news_source.chars().count() > 150 {
            Some("A news source is required (max 150 characters).")
        } else if summary.chars().count() > 500 {
            Some("The summary is too long (max 500 characters).")
        } else if !featured_image.is_empty() && (!featured_image.starts_with("/uploads/") || featured_image.len() > 500) {
            Some("The featured image must be uploaded through the editor.")
        } else if caption.chars().count() > 500 {
            Some("The photo caption is too long (max 500 characters).")
        } else if body.len() > 1_500_000 {
            Some("The article body is too large.")
        } else {
            None
        };

        let input = ArticleInput {
            title,
            summary,
            body,
            kind: article::normalize_kind(&self.kind).to_string(),
            featured_image,
            featured_image_caption: caption,
            news_source,
            category_ids: self.category_ids.clone(),
            sub_category_ids: self.sub_category_ids.clone(),
            tag_ids: self.tag_ids.clone(),
        };
        (input, error)
    }
}

/// Extra rules for submitting / publishing (drafts may be incomplete).
fn incomplete(category_count: usize, body: &str) -> Option<&'static str> {
    if category_count == 0 {
        Some("Select at least one category before submitting or publishing.")
    } else if sanitize::plain_text(body).trim().is_empty() && !body.contains("<img") {
        Some("The article body is empty.")
    } else {
        None
    }
}

// ---------- shared page builders ----------

async fn form_page(
    pool: &MySqlPool,
    session: &Session,
    admin: &CurrentAdmin,
    id: Option<i32>,
    status: &str,
    review_note: &str,
    values: ArticleInput,
    error: Option<String>,
) -> Response {
    let cats = match category::options(pool).await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };
    let names: HashMap<i32, String> = cats.iter().cloned().collect();
    let categories = cats
        .into_iter()
        .map(|(cid, label)| CheckOption { id: cid, label, checked: values.category_ids.contains(&cid) })
        .collect::<Vec<_>>();

    let subs: Vec<(i32, i32, String)> =
        match sqlx::query_as("SELECT id, category_id, name FROM sub_categories ORDER BY category_id, sort_order, name")
            .fetch_all(pool)
            .await
        {
            Ok(s) => s,
            Err(e) => return internal_error(e),
        };
    let mut sub_groups: Vec<SubGroup> = Vec::new();
    for c in &categories {
        let items: Vec<CheckOption> = subs
            .iter()
            .filter(|(_, cid, _)| *cid == c.id)
            .map(|(sid, _, name)| CheckOption { id: *sid, label: name.clone(), checked: values.sub_category_ids.contains(sid) })
            .collect();
        if !items.is_empty() {
            sub_groups.push(SubGroup {
                category_id: c.id,
                category_name: names.get(&c.id).cloned().unwrap_or_default(),
                items,
            });
        }
    }

    let tags = match tag::list(pool).await {
        Ok(t) => t
            .into_iter()
            .map(|t| CheckOption { id: t.id, label: t.name, checked: values.tag_ids.contains(&t.id) })
            .collect(),
        Err(e) => return internal_error(e),
    };

    let kinds = KINDS
        .iter()
        .map(|(value, label)| KindOption { value, label, selected: *value == values.kind })
        .collect();

    let reviewer = is_reviewer(admin);
    let editable_by_author = id.is_none() || matches!(status, "draft" | "changes_requested");

    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }

    render(FormTemplate {
        layout,
        id: id.unwrap_or(0),
        is_edit: id.is_some(),
        status: status.to_string(),
        review_note: review_note.to_string(),
        values,
        categories,
        sub_groups,
        tags,
        kinds,
        is_reviewer: reviewer,
        can_submit: !reviewer && editable_by_author,
    })
}

async fn load(pool: &MySqlPool, id: i32) -> Result<ArticleFull, Response> {
    match article::find(pool, id).await {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err(not_found()),
        Err(e) => Err(internal_error(e)),
    }
}

fn to_show(id: i32) -> Response {
    Redirect::to(&format!("/articles/{id}")).into_response()
}

// ---------- list ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    page: Option<i64>,
    per_page: Option<i64>,
    q: Option<String>,
    kind: Option<String>,
    author: Option<String>, // text on purpose: the form sends "" for "all authors"
    from: Option<String>,
    to: Option<String>,
}

const TABS: [(&str, &str); 5] = [
    ("all", "All"),
    ("draft", "Drafts"),
    ("pending", "Pending review"),
    ("changes_requested", "Changes requested"),
    ("published", "Published"),
];

pub async fn list(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Query(q): Query<ListQuery>,
) -> Response {
    if !can_write(&admin) {
        return forbidden();
    }
    let reviewer = is_reviewer(&admin);
    let requested = q.status.as_deref().unwrap_or(if reviewer { "pending" } else { "all" });
    let status = TABS.iter().map(|t| t.0).find(|k| *k == requested).unwrap_or("all");
    // Journalists only ever see their own articles.
    let author_id = if reviewer { None } else { Some(admin.id) };
    let page = q.page.unwrap_or(1).max(1);
    let per_page = article::normalize_per_page(q.per_page);

    // search / filters
    let q_text: String = q.q.as_deref().unwrap_or("").trim().chars().take(100).collect();
    let kind: String = q
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| KINDS.iter().any(|(v, _)| v == k))
        .unwrap_or("")
        .to_string();
    let author_pick: Option<i32> =
        if reviewer { q.author.as_deref().and_then(|s| s.trim().parse::<i32>().ok()) } else { None };
    let date_from = valid_date(q.from.as_deref());
    let date_to = valid_date(q.to.as_deref());
    let filtered = !q_text.is_empty() || !kind.is_empty() || author_pick.is_some() || !date_from.is_empty() || !date_to.is_empty();
    let extra = format!(
        "&q={}&kind={}&author={}&from={}&to={}",
        enc(&q_text),
        kind,
        author_pick.map(|a| a.to_string()).unwrap_or_default(),
        date_from,
        date_to
    );

    let filter = ListFilter {
        author_id: if reviewer { author_pick } else { author_id },
        status: (status != "all").then(|| status.to_string()),
        q: (!q_text.is_empty()).then(|| q_text.clone()),
        kind: (!kind.is_empty()).then(|| kind.clone()),
        from: (!date_from.is_empty()).then(|| date_from.clone()),
        to: (!date_to.is_empty()).then(|| date_to.clone()),
        page,
        per_page,
    };
    let (rows, total) = match article::list(&state.db_pool, &filter).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let pages = ((total + per_page - 1) / per_page).max(1);
    if page > pages {
        // e.g. an old bookmark after the list got shorter
        return Redirect::to(&format!("/articles?status={status}&per_page={per_page}&page={pages}{extra}")).into_response();
    }

    let counts: HashMap<String, i64> = match article::status_counts(&state.db_pool, author_id).await {
        Ok(c) => c.into_iter().collect(),
        Err(e) => return internal_error(e),
    };
    let all: i64 = counts.values().sum();
    let tabs = TABS
        .iter()
        .map(|(key, label)| StatusTab {
            key,
            label,
            count: if *key == "all" { all } else { counts.get(*key).copied().unwrap_or(0) },
            active: *key == status,
        })
        .collect();
    let sizes = PAGE_SIZES.iter().map(|v| SizeOption { value: *v, active: *v == per_page }).collect();

    let authors: Vec<AuthorOption> = if reviewer {
        match article::authors(&state.db_pool).await {
            Ok(a) => a.into_iter().map(|(id, name)| AuthorOption { id, name, selected: Some(id) == author_pick }).collect(),
            Err(e) => return internal_error(e),
        }
    } else {
        Vec::new()
    };
    let kind_opts = KINDS
        .iter()
        .map(|(value, label)| KindOption { value, label, selected: *value == kind })
        .collect();

    let layout = build_layout(&session, &admin).await;
    render(ListTemplate {
        layout,
        rows,
        tabs,
        status: status.to_string(),
        q: q_text,
        kind_opts,
        authors,
        date_from,
        date_to,
        filtered,
        extra,
        is_reviewer: reviewer,
        page,
        pages,
        total,
        per_page,
        sizes,
        links: page_links(page, pages),
        shown_from: if total == 0 { 0 } else { (page - 1) * per_page + 1 },
        shown_to: (page * per_page).min(total),
    })
}

// ---------- create ----------

pub async fn new_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !can_write(&admin) {
        return forbidden();
    }
    let values = ArticleInput { kind: "news".into(), ..Default::default() };
    form_page(&state.db_pool, &session, &admin, None, "draft", "", values, None).await
}

fn transition_for(action: &str, reviewer: bool, admin_id: i32) -> Option<Transition> {
    match action {
        "" | "save" => Some(Transition::Keep),
        "submit" if !reviewer => Some(Transition::Submit),
        "publish" if reviewer => Some(Transition::Publish(admin_id)),
        _ => None,
    }
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<ArticleForm>,
) -> Response {
    if !can_write(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let Some(transition) = transition_for(&f.action, is_reviewer(&admin), admin.id) else { return forbidden() };

    let (input, error) = f.parse();
    let error = error.or_else(|| {
        if transition != Transition::Keep { incomplete(input.category_ids.len(), &input.body) } else { None }
    });
    if let Some(msg) = error {
        return form_page(&state.db_pool, &session, &admin, None, "draft", "", input, Some(msg.into())).await;
    }

    match article::create(&state.db_pool, admin.id, &input, transition).await {
        Ok(id) => match transition {
            Transition::Keep => {
                flash(&session, "Draft saved.").await;
                Redirect::to(&format!("/articles/{id}/edit")).into_response()
            }
            Transition::Submit => {
                flash(&session, "Submitted for review.").await;
                Redirect::to("/articles").into_response()
            }
            Transition::Publish(_) => {
                flash(&session, "Article published.").await;
                to_show(id)
            }
        },
        Err(e) if is_fk_violation(&e) => {
            let msg = "One of the selected categories, sub categories or tags no longer exists.";
            form_page(&state.db_pool, &session, &admin, None, "draft", "", input, Some(msg.into())).await
        }
        Err(e) => internal_error(e),
    }
}

// ---------- edit ----------

pub async fn edit_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    let art = match load(&state.db_pool, id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !can_view(&admin, art.author_id) {
        return not_found(); // do not reveal other people's articles
    }
    if !can_edit(&admin, art.author_id, &art.status) {
        flash(&session, "This article can no longer be edited. It is waiting for review or already published.").await;
        return to_show(id);
    }

    let (cats, subs, tags) = match article::selected(&state.db_pool, id).await {
        Ok(s) => s,
        Err(e) => return internal_error(e),
    };
    let values = ArticleInput {
        title: art.title.clone(),
        summary: art.summary.clone(),
        body: art.body.clone(),
        kind: art.kind.clone(),
        featured_image: art.featured_image.clone(),
        featured_image_caption: art.featured_image_caption.clone(),
        news_source: art.news_source.clone(),
        category_ids: cats,
        sub_category_ids: subs,
        tag_ids: tags,
    };
    form_page(&state.db_pool, &session, &admin, Some(id), &art.status, &art.review_note, values, None).await
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<ArticleForm>,
) -> Response {
    if !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let art = match load(&state.db_pool, id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !can_view(&admin, art.author_id) {
        return not_found();
    }
    if !can_edit(&admin, art.author_id, &art.status) {
        return forbidden();
    }
    let Some(transition) = transition_for(&f.action, is_reviewer(&admin), admin.id) else { return forbidden() };

    let (input, error) = f.parse();
    let error = error.or_else(|| {
        if transition != Transition::Keep { incomplete(input.category_ids.len(), &input.body) } else { None }
    });
    if let Some(msg) = error {
        return form_page(&state.db_pool, &session, &admin, Some(id), &art.status, &art.review_note, input, Some(msg.into())).await;
    }

    match article::update(&state.db_pool, id, &input, transition).await {
        Ok(()) => match transition {
            Transition::Keep => {
                flash(&session, "Saved.").await;
                Redirect::to(&format!("/articles/{id}/edit")).into_response()
            }
            Transition::Submit => {
                flash(&session, "Submitted for review.").await;
                Redirect::to("/articles").into_response()
            }
            Transition::Publish(_) => {
                flash(&session, "Article published.").await;
                to_show(id)
            }
        },
        Err(e) if is_fk_violation(&e) => {
            let msg = "One of the selected categories, sub categories or tags no longer exists.";
            form_page(&state.db_pool, &session, &admin, Some(id), &art.status, &art.review_note, input, Some(msg.into())).await
        }
        Err(e) => internal_error(e),
    }
}

// ---------- preview / review screen ----------

pub async fn show(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    let art = match load(&state.db_pool, id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !can_view(&admin, art.author_id) {
        return not_found();
    }
    let (categories, sub_categories, tags) = match article::labels(&state.db_pool, id).await {
        Ok(l) => l,
        Err(e) => return internal_error(e),
    };

    let reviewer = is_reviewer(&admin);
    let status = art.status.clone();
    let own = art.author_id == admin.id;
    let layout = build_layout(&session, &admin).await;

    render(ShowTemplate {
        layout,
        status_label: article::status_label(&status),
        categories,
        sub_categories,
        tags,
        can_edit: can_edit(&admin, art.author_id, &status),
        can_submit: !reviewer && own && matches!(status.as_str(), "draft" | "changes_requested"),
        can_publish: reviewer && status != "published",
        can_request_changes: reviewer && status == "pending",
        can_unpublish: reviewer && status == "published",
        can_delete: can_delete(&admin, art.author_id, &status),
        a: art,
    })
}

// ---------- workflow actions ----------

pub async fn submit(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let art = match load(&state.db_pool, id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !(can_write(&admin) && art.author_id == admin.id) {
        return forbidden();
    }
    let linked = match article::selected(&state.db_pool, id).await {
        Ok(s) => s,
        Err(e) => return internal_error(e),
    };
    if let Some(msg) = incomplete(linked.0.len(), &art.body) {
        flash(&session, msg).await;
        return to_show(id);
    }
    match article::submit(&state.db_pool, id).await {
        Ok(true) => flash(&session, "Submitted for review.").await,
        Ok(false) => flash(&session, "This article cannot be submitted in its current state.").await,
        Err(e) => return internal_error(e),
    }
    to_show(id)
}

pub async fn publish(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !is_reviewer(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let art = match load(&state.db_pool, id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let linked = match article::selected(&state.db_pool, id).await {
        Ok(s) => s,
        Err(e) => return internal_error(e),
    };
    if let Some(msg) = incomplete(linked.0.len(), &art.body) {
        flash(&session, msg).await;
        return to_show(id);
    }
    match article::publish(&state.db_pool, id, admin.id).await {
        Ok(true) => flash(&session, "Article published.").await,
        Ok(false) => flash(&session, "This article is already published.").await,
        Err(e) => return internal_error(e),
    }
    to_show(id)
}

#[derive(Deserialize)]
pub struct NoteForm {
    csrf_token: String,
    #[serde(default)]
    note: String,
}

pub async fn request_changes(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<NoteForm>,
) -> Response {
    if !is_reviewer(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let note = f.note.trim();
    if note.is_empty() || note.chars().count() > 1000 {
        flash(&session, "Write a short note for the journalist (max 1000 characters).").await;
        return to_show(id);
    }
    match article::request_changes(&state.db_pool, id, admin.id, note).await {
        Ok(true) => {
            flash(&session, "Sent back to the journalist with your note.").await;
            Redirect::to("/articles").into_response()
        }
        Ok(false) => {
            flash(&session, "Only articles that are pending review can be sent back.").await;
            to_show(id)
        }
        Err(e) => internal_error(e),
    }
}

pub async fn unpublish(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !is_reviewer(&admin) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match article::unpublish(&state.db_pool, id).await {
        Ok(true) => flash(&session, "Article unpublished and removed from the front page.").await,
        Ok(false) => flash(&session, "This article is not published.").await,
        Err(e) => return internal_error(e),
    }
    to_show(id)
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let art = match load(&state.db_pool, id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !can_delete(&admin, art.author_id, &art.status) {
        return forbidden();
    }
    match article::delete(&state.db_pool, id).await {
        Ok(_) => {
            flash(&session, "Article deleted.").await;
            Redirect::to("/articles").into_response()
        }
        Err(e) => internal_error(e),
    }
}