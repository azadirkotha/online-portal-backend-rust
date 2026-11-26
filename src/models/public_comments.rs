//! Visitor comments with a single level of replies.
//! Replying to a reply attaches the new reply to the same top-level comment.
//! Save as src/models/public_comments.rs and add `pub mod public_comments;` to models/mod.rs.

use serde::Serialize;
use sqlx::{FromRow, MySql, MySqlPool, QueryBuilder};
use std::collections::HashMap;

/// Top-level comments per page.
pub const PER_PAGE: i64 = 10;
pub const MAX_NAME: usize = 60;
pub const MAX_BODY: usize = 2000;
/// Set to "pending" to hold new comments back until an editor flips them to "approved".
pub const NEW_STATUS: &str = "approved";

const COLS: &str = "id, parent_id, reply_to, name, body, status,
                    DATE_FORMAT(created_at, '%Y-%m-%dT%H:%i:%sZ') AS created_at";

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Item {
    pub id: i32,
    pub parent_id: Option<i32>,
    pub reply_to: Option<String>,
    pub name: String,
    pub body: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct Thread {
    #[serde(flatten)]
    pub comment: Item,
    pub replies: Vec<Item>,
}

#[derive(Debug, Serialize)]
pub struct CommentPage {
    pub comments: Vec<Thread>,
    pub page: i64,
    pub per_page: i64,
    /// top-level comments (this is what paginates)
    pub total: i64,
    pub total_pages: i64,
    /// top-level comments + replies, for the "মন্তব্য (N)" heading
    pub total_all: i64,
}

pub async fn article_id(pool: &MySqlPool, slug: &str) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM articles WHERE slug = ? AND status = 'published'")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn list(pool: &MySqlPool, article_id: i32, page: i64) -> Result<CommentPage, sqlx::Error> {
    let page = page.max(1);

    // newest top-level comments first
    let sql = format!(
        "SELECT {COLS} FROM comments
          WHERE article_id = ? AND parent_id IS NULL AND status = 'approved'
          ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
    );
    let tops: Vec<Item> = sqlx::query_as::<_, Item>(&sql)
        .bind(article_id)
        .bind(PER_PAGE)
        .bind((page - 1) * PER_PAGE)
        .fetch_all(pool)
        .await?;

    // all replies of those comments in one query, oldest first
    let mut by_parent: HashMap<i32, Vec<Item>> = HashMap::new();
    if !tops.is_empty() {
        let mut qb = QueryBuilder::<MySql>::new(format!(
            "SELECT {COLS} FROM comments WHERE status = 'approved' AND parent_id IN ("
        ));
        let mut sep = qb.separated(", ");
        for t in &tops {
            sep.push_bind(t.id);
        }
        sep.push_unseparated(") ORDER BY created_at ASC, id ASC");
        for r in qb.build_query_as::<Item>().fetch_all(pool).await? {
            if let Some(p) = r.parent_id {
                by_parent.entry(p).or_default().push(r);
            }
        }
    }

    let (total_all, total): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), CAST(COALESCE(SUM(parent_id IS NULL), 0) AS SIGNED)
           FROM comments WHERE article_id = ? AND status = 'approved'",
    )
    .bind(article_id)
    .fetch_one(pool)
    .await?;

    let comments = tops
        .into_iter()
        .map(|c| {
            let replies = by_parent.remove(&c.id).unwrap_or_default();
            Thread { comment: c, replies }
        })
        .collect();

    Ok(CommentPage {
        comments,
        page,
        per_page: PER_PAGE,
        total,
        total_pages: ((total + PER_PAGE - 1) / PER_PAGE).max(1),
        total_all,
    })
}

pub enum AddError {
    /// the comment being replied to does not exist on this article
    NoParent,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AddError {
    fn from(e: sqlx::Error) -> Self {
        AddError::Db(e)
    }
}

/// `parent_id` may be a top-level comment OR a reply: either way the new row is stored
/// under the top-level comment, so the thread never gets deeper than one level.
pub async fn add(
    pool: &MySqlPool,
    article_id: i32,
    parent_id: Option<i32>,
    name: &str,
    body: &str,
) -> Result<Item, AddError> {
    let mut root: Option<i32> = None;
    let mut reply_to: Option<String> = None;

    if let Some(pid) = parent_id {
        let found: Option<(i32, Option<i32>, String)> = sqlx::query_as(
            "SELECT id, parent_id, name FROM comments
              WHERE id = ? AND article_id = ? AND status = 'approved'",
        )
        .bind(pid)
        .bind(article_id)
        .fetch_optional(pool)
        .await?;
        let (id, grand_parent, parent_name) = found.ok_or(AddError::NoParent)?;
        match grand_parent {
            // replying to a reply: attach to its top-level comment and remember who was answered
            Some(top) => {
                root = Some(top);
                reply_to = Some(parent_name);
            }
            None => root = Some(id),
        }
    }

    let res = sqlx::query(
        "INSERT INTO comments (article_id, parent_id, reply_to, name, body, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, UTC_TIMESTAMP())",
    )
    .bind(article_id)
    .bind(root)
    .bind(reply_to)
    .bind(name)
    .bind(body)
    .bind(NEW_STATUS)
    .execute(pool)
    .await?;

    let sql = format!("SELECT {COLS} FROM comments WHERE id = ?");
    let item = sqlx::query_as::<_, Item>(&sql)
        .bind(res.last_insert_id() as i32)
        .fetch_one(pool)
        .await?;
    Ok(item)
}