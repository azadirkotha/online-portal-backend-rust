use sqlx::{MySql, MySqlPool, QueryBuilder};
use std::collections::HashSet;

pub struct Zone {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub capacity: usize,
}

/// The main section of the front page: lead (1), centre grid (6), left column (6).
pub const ZONES: [Zone; 3] = [
    Zone { key: "lead", label: "Lead story", hint: "Big story at the top of the centre column", capacity: 1 },
    Zone { key: "center", label: "Centre column", hint: "Cards under the lead", capacity: 6 },
    Zone { key: "left", label: "Left column", hint: "Compact list on the left", capacity: 6 },
];

#[derive(Debug, sqlx::FromRow)]
pub struct Brief {
    pub id: i32,
    pub title: String,
    pub thumb: String,
    pub published_at: String,
}

const BRIEF: &str = "a.id, a.title, a.featured_image AS thumb,
                     COALESCE(DATE_FORMAT(a.published_at, '%Y-%m-%d %H:%i'), '') AS published_at";

pub async fn zone_items(pool: &MySqlPool, zone: &str) -> Result<Vec<Brief>, sqlx::Error> {
    sqlx::query_as::<_, Brief>(&format!(
        "SELECT {BRIEF} FROM front_page_slots s JOIN articles a ON a.id = s.article_id
          WHERE s.zone = ? AND a.status = 'published' ORDER BY s.position"
    ))
    .bind(zone)
    .fetch_all(pool)
    .await
}

/// Published articles that are not placed anywhere yet (newest first).
pub async fn pool_items(pool: &MySqlPool, limit: i64) -> Result<Vec<Brief>, sqlx::Error> {
    sqlx::query_as::<_, Brief>(&format!(
        "SELECT {BRIEF} FROM articles a
          WHERE a.status = 'published' AND a.id NOT IN (SELECT article_id FROM front_page_slots)
          ORDER BY a.published_at DESC, a.id DESC LIMIT ?"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn published_among(pool: &MySqlPool, ids: &[i32]) -> Result<HashSet<i32>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let mut qb = QueryBuilder::<MySql>::new("SELECT id FROM articles WHERE status = 'published' AND id IN (");
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    sep.push_unseparated(")");
    let found: Vec<i32> = qb.build_query_scalar().fetch_all(pool).await?;
    Ok(found.into_iter().collect())
}

/// Replaces the whole main-section arrangement. Ids must already be validated.
pub async fn save_main(pool: &MySqlPool, zones: &[(&str, Vec<i32>)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM front_page_slots").execute(&mut *tx).await?;
    for (zone, ids) in zones {
        for (pos, id) in ids.iter().enumerate() {
            sqlx::query("INSERT INTO front_page_slots (zone, position, article_id) VALUES (?, ?, ?)")
                .bind(*zone).bind(pos as i32).bind(id).execute(&mut *tx).await?;
        }
    }
    tx.commit().await
}

// ----- categories shown on the front page -----

#[derive(Debug, sqlx::FromRow)]
pub struct HomeCategory {
    pub id: i32,
    pub name: String,
    pub shown: bool,
}

pub async fn home_categories(pool: &MySqlPool) -> Result<Vec<HomeCategory>, sqlx::Error> {
    sqlx::query_as::<_, HomeCategory>(
        "SELECT id, name, show_on_home AS shown FROM categories WHERE is_active = 1
          ORDER BY show_on_home DESC, home_order, sort_order, name",
    )
    .fetch_all(pool)
    .await
}

pub async fn save_home_categories(pool: &MySqlPool, order: &[i32], show: &[i32]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    if !order.is_empty() {
        for (pos, id) in order.iter().enumerate() {
            sqlx::query("UPDATE categories SET home_order = ? WHERE id = ?")
                .bind(pos as i32).bind(id).execute(&mut *tx).await?;
        }
    }
    sqlx::query("UPDATE categories SET show_on_home = 0").execute(&mut *tx).await?;
    for id in show {
        sqlx::query("UPDATE categories SET show_on_home = 1 WHERE id = ? AND is_active = 1")
            .bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await
}

// ----- category leads (position 1 = front-page lead, positions 1 + 2 = category page leads) -----

pub async fn lead_candidates(pool: &MySqlPool, category_id: i32, limit: i64) -> Result<Vec<(i32, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT a.id, CONCAT(a.title, ' (', DATE_FORMAT(a.published_at, '%d %b'), ')')
           FROM articles a JOIN article_categories ac ON ac.article_id = a.id
          WHERE ac.category_id = ? AND a.status = 'published'
          ORDER BY a.published_at DESC, a.id DESC LIMIT ?",
    )
    .bind(category_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// (lead 1, lead 2); 0 = automatic.
pub async fn current_leads(pool: &MySqlPool, category_id: i32) -> Result<(i32, i32), sqlx::Error> {
    let rows: Vec<(i8, i32)> = sqlx::query_as("SELECT position, article_id FROM category_leads WHERE category_id = ?")
        .bind(category_id)
        .fetch_all(pool)
        .await?;
    let mut leads = (0, 0);
    for (pos, id) in rows {
        if pos == 1 {
            leads.0 = id;
        } else if pos == 2 {
            leads.1 = id;
        }
    }
    Ok(leads)
}

pub async fn belongs_to_category(pool: &MySqlPool, article_id: i32, category_id: i32) -> Result<bool, sqlx::Error> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM articles a JOIN article_categories ac ON ac.article_id = a.id
          WHERE a.id = ? AND ac.category_id = ? AND a.status = 'published'",
    )
    .bind(article_id)
    .bind(category_id)
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

pub async fn save_leads(pool: &MySqlPool, category_id: i32, lead1: i32, lead2: i32) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM category_leads WHERE category_id = ?").bind(category_id).execute(&mut *tx).await?;
    for (pos, id) in [(1i8, lead1), (2i8, lead2)] {
        if id > 0 {
            sqlx::query("INSERT INTO category_leads (category_id, position, article_id) VALUES (?, ?, ?)")
                .bind(category_id).bind(pos).bind(id).execute(&mut *tx).await?;
        }
    }
    tx.commit().await
}