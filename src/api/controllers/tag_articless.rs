//! Public API: published articles that carry a given tag.
//!
//!   GET /tags/:slug/articles?page=1&per_page=12
//!
//! `:slug` is the tag's slug. No login needed.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;

use crate::models::tag;
use crate::state::AppState;

const DEFAULT_PER_PAGE: i64 = 12;
const MAX_PER_PAGE: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct TagArticle {
    id: i32,
    title: String,
    slug: String,
    summary: String,
    kind: String,
    featured_image: String,
    featured_image_caption: String,
    published_at: String,
    view_count: i64,
    author_name: String,
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn server_err<E: std::fmt::Debug>(e: E) -> Response {
    eprintln!("tag_articles: {e:?}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "কিছু একটা সমস্যা হয়েছে। আবার চেষ্টা করুন।")
}

pub async fn articles_by_tag(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<PageQuery>,
) -> Result<Response, Response> {
    let slug = slug.trim();
    if slug.is_empty() || slug.chars().count() > 200 {
        return Err(err(StatusCode::NOT_FOUND, "ট্যাগটি পাওয়া যায়নি।"));
    }

    let found = tag::find_by_slug(&state.db_pool, slug).await.map_err(server_err)?;
    let tag = found.ok_or_else(|| err(StatusCode::NOT_FOUND, "ট্যাগটি পাওয়া যায়নি।"))?;

    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);

    // only published articles are ever public
    let mut items: Vec<TagArticle> = sqlx::query_as(
        "SELECT a.id, a.title, a.slug,
                COALESCE(a.summary, '') AS summary,
                CAST(a.kind AS CHAR) AS kind,
                COALESCE(a.featured_image, '') AS featured_image,
                COALESCE(a.featured_image_caption, '') AS featured_image_caption,
                COALESCE(DATE_FORMAT(a.published_at, '%Y-%m-%dT%H:%i:%s'), '') AS published_at,
                CAST(COALESCE(a.view_count, 0) AS SIGNED) AS view_count,
                u.name AS author_name
           FROM article_tags x
           JOIN articles a ON a.id = x.article_id
           JOIN users u ON u.id = a.author_id
          WHERE x.tag_id = ? AND CAST(a.status AS CHAR) = 'published'
          ORDER BY a.published_at DESC, a.id DESC
          LIMIT ? OFFSET ?",
    )
    .bind(tag.id)
    .bind(per_page)
    .bind((page - 1) * per_page)
    .fetch_all(&state.db_pool)
    .await
    .map_err(server_err)?;

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM article_tags x JOIN articles a ON a.id = x.article_id
          WHERE x.tag_id = ? AND CAST(a.status AS CHAR) = 'published'",
    )
    .bind(tag.id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(server_err)?;

    // /uploads/... -> absolute URL, like the other endpoints
    let base = state.config.public_base_url.to_string();
    for a in items.iter_mut() {
        if a.featured_image.starts_with('/') {
            a.featured_image = format!("{base}{}", a.featured_image);
        }
    }

    Ok(Json(json!({
        "tag": tag,
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": ((total + per_page - 1) / per_page).max(1),
    }))
    .into_response())
}