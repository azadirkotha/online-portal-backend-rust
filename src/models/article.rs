use sqlx::{MySql, MySqlPool, QueryBuilder, Transaction};

/// Default page size of the article list.
pub const PER_PAGE: i64 = 10;
/// Page sizes the list can be switched to.
pub const PAGE_SIZES: [i64; 4] = [10, 25, 50, 100];

pub fn normalize_per_page(n: Option<i64>) -> i64 {
    n.filter(|v| PAGE_SIZES.contains(v)).unwrap_or(PER_PAGE)
}

pub const KINDS: [(&str, &str); 3] = [("news", "News"), ("opinion", "Opinion"), ("editorial", "Editorial")];

pub fn normalize_kind(k: &str) -> &'static str {
    KINDS.iter().map(|(v, _)| *v).find(|v| *v == k).unwrap_or("news")
}

pub fn status_label(s: &str) -> &'static str {
    match s {
        "draft" => "Draft",
        "pending" => "Pending review",
        "changes_requested" => "Changes requested",
        "published" => "Published",
        _ => "Unknown",
    }
}

/// What the editor form submits.
#[derive(Debug, Clone, Default)]
pub struct ArticleInput {
    pub title: String,
    pub summary: String,
    pub body: String,
    pub kind: String,
    pub featured_image: String,
    pub featured_image_caption: String,
    pub news_source: String,
    pub category_ids: Vec<i32>,
    pub sub_category_ids: Vec<i32>,
    pub tag_ids: Vec<i32>,
}

/// Status change applied together with a save.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transition {
    Keep,
    Submit,
    Publish(i32), // reviewer id
}

#[derive(Debug, sqlx::FromRow)]
pub struct ArticleRow {
    pub id: i32,
    pub title: String,
    pub status: String,
    pub kind: String,
    pub author_name: String,
    pub updated_at: String,
}

impl ArticleRow {
    pub fn label(&self) -> &'static str {
        status_label(&self.status)
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct ArticleFull {
    pub id: i32,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub kind: String,
    pub featured_image: String,
    pub featured_image_caption: String,
    pub news_source: String,
    pub status: String,
    pub review_note: String,
    pub author_id: i32,
    pub author_name: String,
    pub updated_at: String,
    pub published_at: Option<String>,
}

pub struct ListFilter {
    pub author_id: Option<i32>,
    pub status: Option<String>,
    /// title search
    pub q: Option<String>,
    /// news type: news | opinion | editorial
    pub kind: Option<String>,
    /// last-updated date range, both ends inclusive, as YYYY-MM-DD
    pub from: Option<String>,
    pub to: Option<String>,
    pub page: i64,
    pub per_page: i64,
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

fn push_filters(qb: &mut QueryBuilder<'_, MySql>, f: &ListFilter) {
    if let Some(a) = f.author_id {
        qb.push(" AND a.author_id = ").push_bind(a);
    }
    if let Some(s) = &f.status {
        qb.push(" AND a.status = ").push_bind(s.clone());
    }
    if let Some(q) = &f.q {
        qb.push(" AND a.title LIKE ").push_bind(like(q));
    }
    if let Some(k) = &f.kind {
        qb.push(" AND a.kind = ").push_bind(k.clone());
    }
    if let Some(d) = &f.from {
        qb.push(" AND a.updated_at >= ").push_bind(d.clone());
    }
    if let Some(d) = &f.to {
        // inclusive: everything before the start of the next day
        qb.push(" AND a.updated_at < DATE_ADD(").push_bind(d.clone()).push(", INTERVAL 1 DAY)");
    }
}

/// People who have written at least one article (for the author filter).
pub async fn authors(pool: &MySqlPool) -> Result<Vec<(i32, String)>, sqlx::Error> {
    sqlx::query_as::<_, (i32, String)>(
        "SELECT DISTINCT u.id, u.name FROM users u JOIN articles a ON a.author_id = u.id ORDER BY u.name",
    )
    .fetch_all(pool)
    .await
}

pub async fn list(pool: &MySqlPool, f: &ListFilter) -> Result<(Vec<ArticleRow>, i64), sqlx::Error> {
    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT a.id, a.title, a.status, a.kind, u.name AS author_name,
                DATE_FORMAT(a.updated_at, '%Y-%m-%d %H:%i') AS updated_at
           FROM articles a JOIN users u ON u.id = a.author_id WHERE 1 = 1",
    );
    push_filters(&mut qb, f);
    qb.push(" ORDER BY a.updated_at DESC, a.id DESC LIMIT ")
        .push_bind(f.per_page)
        .push(" OFFSET ")
        .push_bind((f.page - 1) * f.per_page);
    let rows = qb.build_query_as::<ArticleRow>().fetch_all(pool).await?;

    let mut cq = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM articles a WHERE 1 = 1");
    push_filters(&mut cq, f);
    let total: i64 = cq.build_query_scalar().fetch_one(pool).await?;
    Ok((rows, total))
}

pub async fn status_counts(pool: &MySqlPool, author: Option<i32>) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let mut qb = QueryBuilder::<MySql>::new("SELECT status, COUNT(*) FROM articles");
    if let Some(a) = author {
        qb.push(" WHERE author_id = ").push_bind(a);
    }
    qb.push(" GROUP BY status");
    qb.build_query_as::<(String, i64)>().fetch_all(pool).await
}

pub async fn find(pool: &MySqlPool, id: i32) -> Result<Option<ArticleFull>, sqlx::Error> {
    sqlx::query_as::<_, ArticleFull>(
        "SELECT a.id, a.title, a.summary, a.body, a.kind, a.featured_image, a.featured_image_caption,
                COALESCE(a.news_source, '') AS news_source,
                a.status, a.review_note, a.author_id, u.name AS author_name,
                DATE_FORMAT(a.updated_at, '%Y-%m-%d %H:%i') AS updated_at,
                DATE_FORMAT(a.published_at, '%Y-%m-%d %H:%i') AS published_at
           FROM articles a JOIN users u ON u.id = a.author_id WHERE a.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// (category ids, sub category ids, tag ids) linked to the article.
pub async fn selected(pool: &MySqlPool, id: i32) -> Result<(Vec<i32>, Vec<i32>, Vec<i32>), sqlx::Error> {
    let c = sqlx::query_scalar("SELECT category_id FROM article_categories WHERE article_id = ?").bind(id).fetch_all(pool).await?;
    let s = sqlx::query_scalar("SELECT sub_category_id FROM article_sub_categories WHERE article_id = ?").bind(id).fetch_all(pool).await?;
    let t = sqlx::query_scalar("SELECT tag_id FROM article_tags WHERE article_id = ?").bind(id).fetch_all(pool).await?;
    Ok((c, s, t))
}

/// Human-readable (categories, sub categories, tags) for the preview page.
pub async fn labels(pool: &MySqlPool, id: i32) -> Result<(Vec<String>, Vec<String>, Vec<String>), sqlx::Error> {
    let c = sqlx::query_scalar("SELECT c.name FROM article_categories ac JOIN categories c ON c.id = ac.category_id WHERE ac.article_id = ? ORDER BY c.name").bind(id).fetch_all(pool).await?;
    let s = sqlx::query_scalar("SELECT s.name FROM article_sub_categories x JOIN sub_categories s ON s.id = x.sub_category_id WHERE x.article_id = ? ORDER BY s.name").bind(id).fetch_all(pool).await?;
    let t = sqlx::query_scalar("SELECT t.name FROM article_tags x JOIN tags t ON t.id = x.tag_id WHERE x.article_id = ? ORDER BY t.name").bind(id).fetch_all(pool).await?;
    Ok((c, s, t))
}

async fn replace_links(tx: &mut Transaction<'_, MySql>, id: i32, input: &ArticleInput) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM article_categories WHERE article_id = ?").bind(id).execute(&mut **tx).await?;
    sqlx::query("DELETE FROM article_sub_categories WHERE article_id = ?").bind(id).execute(&mut **tx).await?;
    sqlx::query("DELETE FROM article_tags WHERE article_id = ?").bind(id).execute(&mut **tx).await?;

    for cid in &input.category_ids {
        sqlx::query("INSERT IGNORE INTO article_categories (article_id, category_id) VALUES (?, ?)")
            .bind(id).bind(cid).execute(&mut **tx).await?;
    }
    for sid in &input.sub_category_ids {
        sqlx::query("INSERT IGNORE INTO article_sub_categories (article_id, sub_category_id) VALUES (?, ?)")
            .bind(id).bind(sid).execute(&mut **tx).await?;
    }
    for tid in &input.tag_ids {
        sqlx::query("INSERT IGNORE INTO article_tags (article_id, tag_id) VALUES (?, ?)")
            .bind(id).bind(tid).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn apply_transition(tx: &mut Transaction<'_, MySql>, id: i32, tr: Transition) -> Result<(), sqlx::Error> {
    match tr {
        Transition::Keep => {}
        Transition::Submit => {
            sqlx::query("UPDATE articles SET status = 'pending', submitted_at = NOW(), review_note = '' WHERE id = ?")
                .bind(id).execute(&mut **tx).await?;
        }
        Transition::Publish(reviewer) => {
            sqlx::query(
                "UPDATE articles SET status = 'published', published_at = COALESCE(published_at, NOW()),
                        reviewed_by = ?, reviewed_at = NOW(), review_note = '' WHERE id = ?",
            )
            .bind(reviewer).bind(id).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

pub async fn create(pool: &MySqlPool, author_id: i32, input: &ArticleInput, tr: Transition) -> Result<i32, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "INSERT INTO articles (title, slug, summary, body, kind, featured_image, featured_image_caption, news_source, author_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.title)
    .bind(super::slug::slugify(&input.title).chars().take(200).collect::<String>())
    .bind(&input.summary)
    .bind(&input.body)
    .bind(&input.kind)
    .bind(&input.featured_image)
    .bind(&input.featured_image_caption)
    .bind(&input.news_source)
    .bind(author_id)
    .execute(&mut *tx)
    .await?;
    let id = res.last_insert_id() as i32;

    replace_links(&mut tx, id, input).await?;
    apply_transition(&mut tx, id, tr).await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update(pool: &MySqlPool, id: i32, input: &ArticleInput, tr: Transition) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE articles SET title = ?, slug = ?, summary = ?, body = ?, kind = ?,
                featured_image = ?, featured_image_caption = ?, news_source = ? WHERE id = ?",
    )
    .bind(&input.title)
    .bind(super::slug::slugify(&input.title).chars().take(200).collect::<String>())
    .bind(&input.summary)
    .bind(&input.body)
    .bind(&input.kind)
    .bind(&input.featured_image)
    .bind(&input.featured_image_caption)
    .bind(&input.news_source)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    replace_links(&mut tx, id, input).await?;
    apply_transition(&mut tx, id, tr).await?;
    tx.commit().await?;
    Ok(())
}

// ----- status changes (each is guarded in SQL, so two people clicking at once cannot corrupt the flow) -----

pub async fn submit(pool: &MySqlPool, id: i32) -> Result<bool, sqlx::Error> {
    sqlx::query(
        "UPDATE articles SET status = 'pending', submitted_at = NOW(), review_note = ''
          WHERE id = ? AND status IN ('draft', 'changes_requested')",
    )
    .bind(id).execute(pool).await.map(|r| r.rows_affected() > 0)
}

pub async fn publish(pool: &MySqlPool, id: i32, reviewer: i32) -> Result<bool, sqlx::Error> {
    sqlx::query(
        "UPDATE articles SET status = 'published', published_at = COALESCE(published_at, NOW()),
                reviewed_by = ?, reviewed_at = NOW(), review_note = ''
          WHERE id = ? AND status <> 'published'",
    )
    .bind(reviewer).bind(id).execute(pool).await.map(|r| r.rows_affected() > 0)
}

pub async fn request_changes(pool: &MySqlPool, id: i32, reviewer: i32, note: &str) -> Result<bool, sqlx::Error> {
    sqlx::query(
        "UPDATE articles SET status = 'changes_requested', review_note = ?, reviewed_by = ?, reviewed_at = NOW()
          WHERE id = ? AND status = 'pending'",
    )
    .bind(note).bind(reviewer).bind(id).execute(pool).await.map(|r| r.rows_affected() > 0)
}

/// Back to draft; also removes the article from the front page so it can never show while unpublished.
pub async fn unpublish(pool: &MySqlPool, id: i32) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let changed = sqlx::query("UPDATE articles SET status = 'draft', published_at = NULL WHERE id = ? AND status = 'published'")
        .bind(id).execute(&mut *tx).await?.rows_affected() > 0;
    if changed {
        sqlx::query("DELETE FROM front_page_slots WHERE article_id = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM category_leads WHERE article_id = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM header_highlights WHERE article_id = ?").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(changed)
}

pub async fn delete(pool: &MySqlPool, id: i32) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM articles WHERE id = ?").bind(id).execute(pool).await.map(|r| r.rows_affected())
}