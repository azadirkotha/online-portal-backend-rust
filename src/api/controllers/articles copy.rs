use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::models::public_news;
use crate::state::AppState;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>().trim_end())
    }
}

// Percent-encode a slug (Bengali slugs are not URL-safe as they are)
fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn render_share_page(m: &public_news::ShareMeta, share_url: &str, dest: &str, site_name: &str) -> String {
    let title = esc(&m.title);
    let desc = esc(&truncate_chars(m.summary.trim(), 200));
    let share_url = esc(share_url);
    let dest = esc(dest);
    let site_name = esc(site_name);

    let has_image = !m.featured_image.is_empty();
    let card = if has_image { "summary_large_image" } else { "summary" };
    let image = if has_image {
        let img = esc(&m.featured_image);
        let alt = esc(&m.featured_image_caption);
        format!(
            r#"<meta property="og:image" content="{img}">
<meta property="og:image:alt" content="{alt}">
<meta name="twitter:image" content="{img}">
"#
        )
    } else {
        String::new()
    };

    format!(
        r#"<!doctype html>
<html lang="bn">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<meta name="description" content="{desc}">
<link rel="canonical" href="{share_url}">
<meta property="og:type" content="article">
<meta property="og:site_name" content="{site_name}">
<meta property="og:locale" content="bn_BD">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{desc}">
<meta property="og:url" content="{share_url}">
{image}<meta name="twitter:card" content="{card}">
<meta name="twitter:title" content="{title}">
<meta name="twitter:description" content="{desc}">
</head>
<body>
<p><a id="go" href="{dest}">{title}</a></p>
<script>window.location.replace(document.getElementById("go").href);</script>
</body>
</html>"#
    )
}

pub async fn share(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let base = state.config.public_base_url.to_string();
    match public_news::share_meta(&state.db_pool, &base, &slug).await {
        Ok(Some(m)) => {
            let enc = pct_encode(&m.slug);
            let share_url = format!("{}/share/{}", base.trim_end_matches('/'), enc);
            let site = std::env::var("SITE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
            let dest = format!("{}/news/{}", site.trim_end_matches('/'), enc);
            let site_name = std::env::var("SITE_NAME").unwrap_or_else(|_| "News Portal".to_string());
            let html = render_share_page(&m, &share_url, &dest, &site_name);
            ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
        }
        Ok(None) => not_found("Article"),
        Err(e) => server_error(e),
    }
}

#[derive(Deserialize)]
pub struct PageQuery {
    /// sub category slug (the "বিষয় অনুযায়ী খবর" chips)
    pub sub: Option<String>,
    pub page: Option<i64>,
}

fn server_error(e: sqlx::Error) -> Response {
    eprintln!("articles api error: {e:?}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Something went wrong"}))).into_response()
}

fn not_found(what: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": format!("{what} not found")}))).into_response()
}

/// GET /api/home : everything the landing page needs in one call
pub async fn home(State(state): State<AppState>) -> Response {
    match public_news::home(&state.db_pool, &state.config.public_base_url).await {
        Ok(h) => Json(h).into_response(),
        Err(e) => server_error(e),
    }
}

/// GET /api/categories/:slug/page?sub=<sub slug>&page=1
pub async fn category_page(State(state): State<AppState>, Path(slug): Path<String>, Query(q): Query<PageQuery>) -> Response {
    let sub = q.sub.filter(|s| !s.trim().is_empty());
    match public_news::category_page(&state.db_pool, &state.config.public_base_url, &slug, sub, q.page.unwrap_or(1)).await {
        Ok(Some(p)) => Json(p).into_response(),
        Ok(None) => not_found("Category"),
        Err(e) => server_error(e),
    }
}

/// GET /api/tags/:slug/articles?page=1
pub async fn tag_articles(State(state): State<AppState>, Path(slug): Path<String>, Query(q): Query<PageQuery>) -> Response {
    match public_news::tag_page(&state.db_pool, &state.config.public_base_url, &slug, q.page.unwrap_or(1)).await {
        Ok(Some(p)) => Json(p).into_response(),
        Ok(None) => not_found("Tag"),
        Err(e) => server_error(e),
    }
}

/// GET /api/articles/:id
pub async fn detail(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    match public_news::article(&state.db_pool, &state.config.public_base_url, &slug).await {
        Ok(Some(a)) => Json(a).into_response(),
        Ok(None) => not_found("Article"),
        Err(e) => server_error(e),
    }
}


#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub cat: Option<String>,
    pub date: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn search(State(state): State<AppState>, Query(p): Query<SearchQuery>) -> Response {
    let q = p.q.unwrap_or_default();
    let cat = p.cat.filter(|s| !s.trim().is_empty());
    let date = p.date.filter(|s| !s.trim().is_empty());
    let offset = p.offset.unwrap_or(0).max(0);
    let limit = p.limit.unwrap_or(12).clamp(1, 50);

    match public_news::search(&state.db_pool, &state.config.public_base_url, &q, cat, date, offset, limit).await {
        Ok(page) => Json(page).into_response(),
        Err(e) => server_error(e),
    }
}

pub async fn search_categories(State(state): State<AppState>) -> Response {
    match public_news::search_categories(&state.db_pool).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => server_error(e),
    }
}