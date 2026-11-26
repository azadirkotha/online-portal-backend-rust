use serde::Serialize;
use sqlx::{MySql, MySqlPool, QueryBuilder};

pub const PER_PAGE: i64 = 10;

#[derive(Debug, Clone, Default)]
pub struct BreakingInput {
    pub headline: String,
    pub path: String,
    pub is_active: bool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct BreakingRow {
    pub id: i32,
    pub headline: String,
    pub path: String,
    pub is_active: bool,
    pub created_at: String,
}

/// `%text%` for LIKE, with the LIKE wildcards in the text escaped.
fn like(s: &str) -> String {
    let mut out = String::from("%");
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

pub async fn list(pool: &MySqlPool, q: &str, page: i64) -> Result<(Vec<BreakingRow>, i64), sqlx::Error> {
    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT id, headline, path, is_active,
                DATE_FORMAT(created_at, '%Y-%m-%d %H:%i') AS created_at
           FROM breaking_news WHERE 1 = 1",
    );
    if !q.is_empty() {
        qb.push(" AND headline LIKE ").push_bind(like(q));
    }
    qb.push(" ORDER BY created_at DESC, id DESC LIMIT ")
        .push_bind(PER_PAGE)
        .push(" OFFSET ")
        .push_bind((page - 1) * PER_PAGE);
    let rows = qb.build_query_as::<BreakingRow>().fetch_all(pool).await?;

    let mut cq = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM breaking_news WHERE 1 = 1");
    if !q.is_empty() {
        cq.push(" AND headline LIKE ").push_bind(like(q));
    }
    let total: i64 = cq.build_query_scalar().fetch_one(pool).await?;
    Ok((rows, total))
}

pub async fn active_count(pool: &MySqlPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM breaking_news WHERE is_active = 1")
        .fetch_one(pool)
        .await
}

pub async fn find(pool: &MySqlPool, id: i32) -> Result<Option<BreakingRow>, sqlx::Error> {
    sqlx::query_as::<_, BreakingRow>(
        "SELECT id, headline, path, is_active, DATE_FORMAT(created_at, '%Y-%m-%d %H:%i') AS created_at
           FROM breaking_news WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn create(pool: &MySqlPool, admin_id: i32, i: &BreakingInput) -> Result<i32, sqlx::Error> {
    let res = sqlx::query("INSERT INTO breaking_news (headline, path, is_active, created_by) VALUES (?, ?, ?, ?)")
        .bind(&i.headline)
        .bind(&i.path)
        .bind(i.is_active)
        .bind(admin_id)
        .execute(pool)
        .await?;
    Ok(res.last_insert_id() as i32)
}

pub async fn update(pool: &MySqlPool, id: i32, i: &BreakingInput) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE breaking_news SET headline = ?, path = ?, is_active = ? WHERE id = ?")
        .bind(&i.headline)
        .bind(&i.path)
        .bind(i.is_active)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &MySqlPool, id: i32) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM breaking_news WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

/// Bulk checkbox save for the rows visible on the current page:
/// every row in `page_ids` is switched off, then the ones in `checked` are switched on.
/// Rows on other pages are untouched.
pub async fn set_active_on_page(pool: &MySqlPool, page_ids: &[i32], checked: &[i32]) -> Result<(), sqlx::Error> {
    if page_ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;

    let mut off = QueryBuilder::<MySql>::new("UPDATE breaking_news SET is_active = 0 WHERE id IN (");
    let mut sep = off.separated(", ");
    for id in page_ids {
        sep.push_bind(*id);
    }
    sep.push_unseparated(")");
    off.build().execute(&mut *tx).await?;

    let on_ids: Vec<i32> = checked.iter().copied().filter(|id| page_ids.contains(id)).collect();
    if !on_ids.is_empty() {
        let mut on = QueryBuilder::<MySql>::new("UPDATE breaking_news SET is_active = 1 WHERE id IN (");
        let mut sep = on.separated(", ");
        for id in &on_ids {
            sep.push_bind(*id);
        }
        sep.push_unseparated(")");
        on.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

// ---------- public ----------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BreakingItem {
    pub id: i32,
    pub headline: String,
    pub path: String,
}

/// Only the checked (active) headlines, newest first.
pub async fn public_list(pool: &MySqlPool, limit: i64) -> Result<Vec<BreakingItem>, sqlx::Error> {
    sqlx::query_as::<_, BreakingItem>(
        "SELECT id, headline, path FROM breaking_news
          WHERE is_active = 1 ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}