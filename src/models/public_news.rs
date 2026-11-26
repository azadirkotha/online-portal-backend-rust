//! Read-only queries behind the public JSON API (published articles only).

use serde::Serialize;
use sqlx::{MySql, MySqlPool, QueryBuilder};
use chrono::{Duration, FixedOffset, NaiveDateTime, NaiveDate, Utc};
use std::collections::HashMap;

use super::category::{self, PublicCategory};
use super::front_page::ZONES;
use super::tag::{self, Tag};

/// Articles shown under the lead in each category section on the front page.
pub const SECTION_OTHERS: i64 = 8;
pub const PAGE_SIZE: i64 = 12;

// Timezone that defines "today" for readers (Bangladesh = +6)
const SITE_TZ_OFFSET_HOURS: i32 = 6;
// Timezone `articles.published_at` is stored in: 0 if it holds UTC, 6 if it holds Bangladesh local time
const DB_TZ_OFFSET_HOURS: i32 = 0;
// Max number of "today" stories returned on page 1
const TODAY_LIMIT: i64 = 40;

/// Midnight (start of today in the site timezone), expressed in the timezone the DB column uses.
fn today_start() -> NaiveDateTime {
    let site = FixedOffset::east_opt(SITE_TZ_OFFSET_HOURS * 3600).unwrap();
    let midnight = Utc::now()
        .with_timezone(&site)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    midnight + Duration::hours((DB_TZ_OFFSET_HOURS - SITE_TZ_OFFSET_HOURS) as i64)
}

/// today == true  -> published_at >= boundary
/// today == false -> published_at <  boundary
fn published_range(qb: &mut QueryBuilder<'_, MySql>, boundary: NaiveDateTime, today: bool) {
    qb.push(if today { " AND a.published_at >= " } else { " AND a.published_at < " });
    qb.push_bind(boundary.format("%Y-%m-%d %H:%M:%S").to_string());
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Card {
    pub id: i32,
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub featured_image: String,
    pub featured_image_caption: String,
    pub kind: String,
    pub published_at: Option<String>,
}

/// id / name / slug: used for categories, sub categories and tags.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Ref {
    pub id: i32,
    pub name: String,
    pub slug: String,
}

const CARD: &str = "a.id, a.title, a.slug, a.summary, a.featured_image, a.featured_image_caption, a.kind,
                    DATE_FORMAT(a.published_at, '%Y-%m-%dT%H:%i:%s') AS published_at";

// ---------- small query helpers ----------

fn select() -> QueryBuilder<'static, MySql> {
    QueryBuilder::new(format!("SELECT {CARD} FROM articles a WHERE a.status = 'published'"))
}

fn exclude(qb: &mut QueryBuilder<'_, MySql>, ids: &[i32]) {
    if ids.is_empty() {
        return;
    }
    qb.push(" AND a.id NOT IN (");
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    sep.push_unseparated(")");
}

fn in_category(qb: &mut QueryBuilder<'_, MySql>, category_id: i32) {
    qb.push(" AND EXISTS (SELECT 1 FROM article_categories ac WHERE ac.article_id = a.id AND ac.category_id = ")
        .push_bind(category_id)
        .push(")");
}

fn with_sub(qb: &mut QueryBuilder<'_, MySql>, category_id: i32, sub: &Option<String>) {
    if let Some(s) = sub {
        qb.push(
            " AND EXISTS (SELECT 1 FROM article_sub_categories x JOIN sub_categories sc ON sc.id = x.sub_category_id
               WHERE x.article_id = a.id AND sc.category_id = ",
        )
        .push_bind(category_id)
        .push(" AND sc.slug = ")
        .push_bind(s.clone())
        .push(")");
    }
}

const NEWEST: &str = " ORDER BY a.published_at DESC, a.id DESC";

fn absolute(base: &str, mut c: Card) -> Card {
    if c.featured_image.starts_with('/') {
        c.featured_image = format!("{base}{}", c.featured_image);
    }
    c
}

async fn run(pool: &MySqlPool, mut qb: QueryBuilder<'_, MySql>, base: &str) -> Result<Vec<Card>, sqlx::Error> {
    let rows = qb.build_query_as::<Card>().fetch_all(pool).await?;
    Ok(rows.into_iter().map(|c| absolute(base, c)).collect())
}

async fn cards_by_ids(pool: &MySqlPool, base: &str, ids: &[i32]) -> Result<Vec<Card>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb = select();
    qb.push(" AND a.id IN (");
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    sep.push_unseparated(")");
    let mut by_id: HashMap<i32, Card> = run(pool, qb, base).await?.into_iter().map(|c| (c.id, c)).collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect()) // keep the requested order
}

async fn latest(pool: &MySqlPool, base: &str, limit: i64, excl: &[i32]) -> Result<Vec<Card>, sqlx::Error> {
    let mut qb = select();
    exclude(&mut qb, excl);
    qb.push(NEWEST).push(" LIMIT ").push_bind(limit);
    run(pool, qb, base).await
}

async fn latest_of_kind(pool: &MySqlPool, base: &str, kind: &str, limit: i64) -> Result<Vec<Card>, sqlx::Error> {
    let mut qb = select();
    qb.push(" AND a.kind = ").push_bind(kind.to_string()).push(NEWEST).push(" LIMIT ").push_bind(limit);
    run(pool, qb, base).await
}

/// Hand-picked leads for a category (positions 1..=n) topped up with the newest articles.
/// `excl` = article ids that must not be used (e.g. already shown in the upper front-page sections).
async fn category_leads(
    pool: &MySqlPool,
    base: &str,
    category_id: i32,
    n: i64,
    excl: &[i32],
) -> Result<Vec<Card>, sqlx::Error> {
    let mut picked: Vec<i32> = sqlx::query_scalar(
        "SELECT cl.article_id FROM category_leads cl
           JOIN articles a ON a.id = cl.article_id AND a.status = 'published'
           JOIN article_categories ac ON ac.article_id = a.id AND ac.category_id = cl.category_id
          WHERE cl.category_id = ? AND cl.position <= ? ORDER BY cl.position",
    )
    .bind(category_id)
    .bind(n)
    .fetch_all(pool)
    .await?;

    // a hand-picked lead that is already shown above is skipped
    picked.retain(|id| !excl.contains(id));

    let mut leads = cards_by_ids(pool, base, &picked).await?;
    let missing = n - leads.len() as i64;
    if missing > 0 {
        let mut skip: Vec<i32> = excl.to_vec();
        skip.extend(picked.iter().copied());
        let mut qb = select();
        in_category(&mut qb, category_id);
        exclude(&mut qb, &skip);
        qb.push(NEWEST).push(" LIMIT ").push_bind(missing);
        leads.extend(run(pool, qb, base).await?);
    }
    Ok(leads)
}

/// The stories for the header bar: the editors' picks (today's portal only), topped up with the newest.
/// `before` = exclusive upper bound on published_at, e.g. "2026-11-01" for the 31 October portal.
pub async fn header_featured(pool: &MySqlPool, base: &str, limit: i64, before: Option<&str>) -> Result<Vec<Card>, sqlx::Error> {
    let mut cards = if before.is_none() {
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT h.article_id FROM header_highlights h
               JOIN articles a ON a.id = h.article_id AND a.status = 'published'
              ORDER BY h.position",
        )
        .fetch_all(pool)
        .await?;
        cards_by_ids(pool, base, &ids).await?
    } else {
        Vec::new()
    };
    cards.truncate(limit as usize);

    let missing = limit - cards.len() as i64;
    if missing > 0 {
        let used: Vec<i32> = cards.iter().map(|c| c.id).collect();
        let mut qb = select();
        exclude(&mut qb, &used);
        if let Some(cut) = before {
            qb.push(" AND a.published_at < ").push_bind(cut.to_string());
        }
        qb.push(NEWEST).push(" LIMIT ").push_bind(missing);
        cards.extend(run(pool, qb, base).await?);
    }
    Ok(cards)
}

// ---------- front page ----------

#[derive(Debug, Serialize)]
pub struct HomeSection {
    pub category: Ref,
    pub lead: Option<Card>,
    pub articles: Vec<Card>,
}

#[derive(Debug, Serialize)]
pub struct HomePage {
    pub lead: Option<Card>,
    pub center: Vec<Card>,
    pub left: Vec<Card>,
    pub popular: Vec<Card>,
    pub editorials: Vec<Card>,
    pub opinions: Vec<Card>,
    pub sections: Vec<HomeSection>,
}

pub async fn home(pool: &MySqlPool, base: &str) -> Result<HomePage, sqlx::Error> {
    // 1. what the editors arranged
    let slots: Vec<(String, i32)> = sqlx::query_as(
        "SELECT s.zone, s.article_id FROM front_page_slots s JOIN articles a ON a.id = s.article_id
          WHERE a.status = 'published' ORDER BY s.zone, s.position",
    )
    .fetch_all(pool)
    .await?;
    let used: Vec<i32> = slots.iter().map(|s| s.1).collect();
    let mut by_id: HashMap<i32, Card> =
        cards_by_ids(pool, base, &used).await?.into_iter().map(|c| (c.id, c)).collect();

    let mut zones: Vec<Vec<Card>> = ZONES
        .iter()
        .map(|z| slots.iter().filter(|s| s.0 == z.key).filter_map(|s| by_id.remove(&s.1)).take(z.capacity).collect())
        .collect();

    // 2. empty slots are topped up with the newest articles so the page is never half empty
    let missing: usize = ZONES.iter().zip(&zones).map(|(z, v)| z.capacity - v.len()).sum();
    if missing > 0 {
        let mut extra = latest(pool, base, missing as i64, &used).await?.into_iter();
        for (z, v) in ZONES.iter().zip(zones.iter_mut()) {
            while v.len() < z.capacity {
                match extra.next() {
                    Some(c) => v.push(c),
                    None => break,
                }
            }
        }
    }
    let mut zones = zones.into_iter();
    let lead = zones.next().and_then(|v| v.into_iter().next());
    let center = zones.next().unwrap_or_default();
    let left = zones.next().unwrap_or_default();

    // Everything shown in the upper sections (lead, center, left) is kept out of the category sections.
    let mut top_ids: Vec<i32> = lead.iter().map(|c| c.id).collect();
    top_ids.extend(center.iter().map(|c| c.id));
    top_ids.extend(left.iter().map(|c| c.id));

    // 3. right column: popular (last 7 days by views), editorials, opinions
    let mut qb = select();
    qb.push(" AND a.published_at >= NOW() - INTERVAL 7 DAY ORDER BY a.view_count DESC, a.published_at DESC LIMIT 5");
    let mut popular = run(pool, qb, base).await?;
    if popular.is_empty() {
        popular = latest(pool, base, 5, &[]).await?;
    }
    let editorials = latest_of_kind(pool, base, "editorial", 5).await?;
    let opinions = latest_of_kind(pool, base, "opinion", 5).await?;

    // 4. category sections chosen by the editors (stories already shown above are left out)
    let cats: Vec<Ref> = sqlx::query_as(
        "SELECT id, name, slug FROM categories WHERE show_on_home = 1 AND is_active = 1 ORDER BY home_order, name",
    )
    .fetch_all(pool)
    .await?;
    let mut sections = Vec::new();
    for cat in cats {
        let sec_lead = category_leads(pool, base, cat.id, 1, &top_ids).await?.into_iter().next();

        let mut skip: Vec<i32> = top_ids.clone();
        skip.extend(sec_lead.iter().map(|l| l.id));

        let mut qb = select();
        in_category(&mut qb, cat.id);
        exclude(&mut qb, &skip);
        qb.push(NEWEST).push(" LIMIT ").push_bind(SECTION_OTHERS);
        let articles = run(pool, qb, base).await?;
        sections.push(HomeSection { category: cat, lead: sec_lead, articles });
    }

    Ok(HomePage { lead, center, left, popular, editorials, opinions, sections })
}

// ---------- category page ----------

#[derive(Debug, Serialize)]
pub struct CategoryPage {
    pub category: PublicCategory,
    pub active_sub: Option<String>,
    /// Two lead stories (first page only)
    pub leads: Vec<Card>,
    pub today: Vec<Card>,    // new: same type as the old `articles`
    pub older: Vec<Card>,    // renamed from `articles`
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub total_pages: i64,
}

pub async fn category_page(
    pool: &MySqlPool,
    base: &str,
    slug: &str,
    sub: Option<String>,
    page: i64,
) -> Result<Option<CategoryPage>, sqlx::Error> {
    let Some(cat) = category::public_by_slug(pool, slug).await? else { return Ok(None) };
    let page = page.max(1);
    let boundary = today_start();

    // the category page itself shows everything, so nothing is excluded here
    let leads = category_leads(pool, base, cat.id, 1, &[]).await?;
    let lead_ids: Vec<i32> = leads.iter().map(|l| l.id).collect();

    // Today's news (page 1 only, newest first, lead excluded)
    let today = if page == 1 {
        let mut qb = select();
        in_category(&mut qb, cat.id);
        exclude(&mut qb, &lead_ids);
        with_sub(&mut qb, cat.id, &sub);
        published_range(&mut qb, boundary, true);
        qb.push(NEWEST).push(" LIMIT ").push_bind(TODAY_LIMIT);
        run(pool, qb, base).await?
    } else {
        Vec::new()
    };

    // Older news (before today), paginated
    let mut qb = select();
    in_category(&mut qb, cat.id);
    exclude(&mut qb, &lead_ids);
    with_sub(&mut qb, cat.id, &sub);
    published_range(&mut qb, boundary, false);
    qb.push(NEWEST).push(" LIMIT ").push_bind(PAGE_SIZE).push(" OFFSET ").push_bind((page - 1) * PAGE_SIZE);
    let older = run(pool, qb, base).await?;

    // Total counts the older list only (it is the one that paginates)
    let mut cq = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM articles a WHERE a.status = 'published'");
    in_category(&mut cq, cat.id);
    exclude(&mut cq, &lead_ids);
    with_sub(&mut cq, cat.id, &sub);
    published_range(&mut cq, boundary, false);
    let total: i64 = cq.build_query_scalar().fetch_one(pool).await?;

    Ok(Some(CategoryPage {
        category: cat,
        active_sub: sub,
        leads: if page == 1 { leads } else { Vec::new() },
        today,
        older,
        page,
        per_page: PAGE_SIZE,
        total,
        total_pages: ((total + PAGE_SIZE - 1) / PAGE_SIZE).max(1),
    }))
}

// ---------- tag page ----------

#[derive(Debug, Serialize)]
pub struct TagPage {
    pub tag: Tag,
    pub articles: Vec<Card>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub total_pages: i64,
}

pub async fn tag_page(pool: &MySqlPool, base: &str, slug: &str, page: i64) -> Result<Option<TagPage>, sqlx::Error> {
    let Some(t) = tag::find_by_slug(pool, slug).await? else { return Ok(None) };
    let page = page.max(1);

    let mut qb = select();
    qb.push(" AND EXISTS (SELECT 1 FROM article_tags x WHERE x.article_id = a.id AND x.tag_id = ")
        .push_bind(t.id)
        .push(")")
        .push(NEWEST)
        .push(" LIMIT ")
        .push_bind(PAGE_SIZE)
        .push(" OFFSET ")
        .push_bind((page - 1) * PAGE_SIZE);
    let articles = run(pool, qb, base).await?;

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM articles a JOIN article_tags x ON x.article_id = a.id
          WHERE a.status = 'published' AND x.tag_id = ?",
    )
    .bind(t.id)
    .fetch_one(pool)
    .await?;

    Ok(Some(TagPage {
        tag: t,
        articles,
        page,
        per_page: PAGE_SIZE,
        total,
        total_pages: ((total + PAGE_SIZE - 1) / PAGE_SIZE).max(1),
    }))
}

// ---------- article page ----------

#[derive(Debug, sqlx::FromRow)]
struct DetailRow {
    id: i32,
    title: String,
    slug: String,
    summary: String,
    body: String,
    kind: String,
    featured_image: String,
    featured_image_caption: String,
    published_at: Option<String>,
    author: String,
    author_photo: String,
    author_designation: String,
    author_bio: String,
    view_count: i32,
    news_source: String,
}

#[derive(Debug, Serialize)]
pub struct ArticleDetail {
    pub id: i32,
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub body: String,
    pub kind: String,
    pub featured_image: String,
    pub featured_image_caption: String,
    pub published_at: Option<String>,
    pub author: String,
    pub author_photo: String,
    pub author_designation: String,
    pub author_bio: String,
    pub view_count: i32,
    pub categories: Vec<Ref>,
    pub sub_categories: Vec<Ref>,
    pub tags: Vec<Ref>,
    pub related: Vec<RelatedArticle>,
    pub news_source: String,
}

use sqlx::FromRow;

#[derive(Debug, Serialize, FromRow)]
pub struct RelatedArticle {
    pub id: i32,
    pub title: String,
    pub slug: String,
    pub featured_image: String,
    pub published_at: String,
}

/// Writes buffered page views to the database (one UPDATE per viewed article).
pub async fn add_views(pool: &MySqlPool, counts: &HashMap<String, u32>) {
    for (slug, n) in counts {
        if let Err(e) = sqlx::query(
            "UPDATE articles SET view_count = view_count + ? WHERE slug = ? AND status = 'published'",
        )
        .bind(*n)
        .bind(slug)
        .execute(pool)
        .await
        {
            eprintln!("view count flush failed for {slug}: {e:?}");
        }
    }
}

pub async fn article(pool: &MySqlPool, base: &str, slug: &str) -> Result<Option<ArticleDetail>, sqlx::Error> {
    let row = sqlx::query_as::<_, DetailRow>(
        "SELECT a.id, a.title, a.slug, a.summary, a.body, a.kind, a.featured_image, a.featured_image_caption,
                DATE_FORMAT(a.published_at, '%Y-%m-%dT%H:%i:%s') AS published_at,
                u.name AS author, u.photo AS author_photo, u.designation AS author_designation, u.bio AS author_bio,
                a.view_count,
                COALESCE(a.news_source, '') AS news_source
           FROM articles a JOIN users u ON u.id = a.author_id
          WHERE a.slug = ? AND a.status = 'published'",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let id = r.id;

    let categories = sqlx::query_as("SELECT c.id, c.name, c.slug FROM article_categories x JOIN categories c ON c.id = x.category_id WHERE x.article_id = ? ORDER BY c.name").bind(id).fetch_all(pool).await?;
    let sub_categories = sqlx::query_as("SELECT s.id, s.name, s.slug FROM article_sub_categories x JOIN sub_categories s ON s.id = x.sub_category_id WHERE x.article_id = ? ORDER BY s.name").bind(id).fetch_all(pool).await?;
    let tags = sqlx::query_as("SELECT t.id, t.name, t.slug FROM article_tags x JOIN tags t ON t.id = x.tag_id WHERE x.article_id = ? ORDER BY t.name").bind(id).fetch_all(pool).await?;

    let abs = |p: String| if p.starts_with('/') { format!("{base}{p}") } else { p };

    // Latest 5 published stories that share at least one category with this one
    let related = sqlx::query_as::<_, RelatedArticle>(
        "SELECT a.id, a.title, a.slug, COALESCE(a.featured_image, '') AS featured_image,
                DATE_FORMAT(a.published_at, '%Y-%m-%dT%H:%i:%s') AS published_at
           FROM articles a
          WHERE a.status = 'published'
            AND a.id <> ?
            AND EXISTS (
                SELECT 1
                  FROM article_categories mine
                  JOIN article_categories theirs ON theirs.category_id = mine.category_id
                 WHERE mine.article_id = ? AND theirs.article_id = a.id
            )
          ORDER BY a.published_at DESC, a.id DESC
          LIMIT 5",
    )
    .bind(id)
    .bind(id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|mut x| {
        x.featured_image = abs(x.featured_image);
        x
    })
    .collect::<Vec<_>>();

    Ok(Some(ArticleDetail {
        id: r.id,
        title: r.title,
        slug: r.slug,
        summary: r.summary,
        body: r.body.replace("src=\"/uploads/", &format!("src=\"{base}/uploads/")),
        kind: r.kind,
        featured_image: abs(r.featured_image),
        featured_image_caption: r.featured_image_caption,
        published_at: r.published_at,
        author: r.author,
        author_photo: abs(r.author_photo),
        author_designation: r.author_designation,
        author_bio: r.author_bio,
        view_count: r.view_count,
        news_source: r.news_source,
        categories,
        sub_categories,
        tags,
        related,
    }))
}

#[derive(Debug, FromRow)]
pub struct ShareMeta {
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub featured_image: String,
    pub featured_image_caption: String,
}

pub async fn share_meta(pool: &MySqlPool, base: &str, slug: &str) -> Result<Option<ShareMeta>, sqlx::Error> {
    let row = sqlx::query_as::<_, ShareMeta>(
        "SELECT a.title, a.slug,
                COALESCE(a.summary, '') AS summary,
                COALESCE(a.featured_image, '') AS featured_image,
                COALESCE(a.featured_image_caption, '') AS featured_image_caption
           FROM articles a
          WHERE a.slug = ? AND a.status = 'published'",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|mut m| {
        if m.featured_image.starts_with('/') {
            m.featured_image = format!("{base}{}", m.featured_image);
        }
        m
    }))
}

// Search 

const SEARCH_MAX_TERMS: usize = 6;
const SEARCH_MAX_LEN: usize = 100;

#[derive(Debug, Serialize, FromRow)]
pub struct SearchItem {
    pub id: i32,
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub featured_image: String,
    pub published_at: String,
    pub category_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct SearchCategory {
    pub id: i32,
    pub name: String,
    pub slug: String,
}

// "%word%" with LIKE wildcards in the user's text escaped
fn like_pattern(s: &str) -> String {
    let mut out = String::from("%");
    for ch in s.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

// Every word must match the title, summary or body; optional category and "before this day" filters
fn push_search_filters(
    qb: &mut QueryBuilder<'_, MySql>,
    terms: &[String],
    cat: &Option<String>,
    before: &Option<String>,
) {
    for t in terms {
        let pat = like_pattern(t);
        qb.push(" AND (a.title LIKE ")
            .push_bind(pat.clone())
            .push(" OR a.summary LIKE ")
            .push_bind(pat.clone())
            .push(" OR a.body LIKE ")
            .push_bind(pat)
            .push(")");
    }
    if let Some(slug) = cat {
        qb.push(" AND EXISTS (SELECT 1 FROM article_categories x JOIN categories c ON c.id = x.category_id WHERE x.article_id = a.id AND c.slug = ")
            .push_bind(slug.clone())
            .push(")");
    }
    if let Some(day) = before {
        qb.push(" AND a.published_at < ").push_bind(day.clone());
    }
}

pub async fn search(
    pool: &MySqlPool,
    base: &str,
    q: &str,
    cat: Option<String>,
    date: Option<String>,
    offset: i64,
    limit: i64,
) -> Result<SearchPage, sqlx::Error> {
    let q: String = q.trim().chars().take(SEARCH_MAX_LEN).collect();
    let terms: Vec<String> = q.split_whitespace().take(SEARCH_MAX_TERMS).map(str::to_string).collect();

    // nothing to search for: return an empty page instead of the whole archive
    if terms.is_empty() && cat.is_none() {
        return Ok(SearchPage { items: Vec::new(), total: 0, offset, limit });
    }

    // archive day (YYYY-MM-DD): only stories published before the end of that day
    let before = date
        .and_then(|d| NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok())
        .map(|d| format!("{} 00:00:00", d + chrono::Duration::days(1)));

    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT a.id, a.title, a.slug,
                COALESCE(a.summary, '') AS summary,
                COALESCE(a.featured_image, '') AS featured_image,
                DATE_FORMAT(a.published_at, '%Y-%m-%dT%H:%i:%s') AS published_at,
                (SELECT c.name FROM article_categories x JOIN categories c ON c.id = x.category_id
                  WHERE x.article_id = a.id ORDER BY ",
    );
    // when filtering by category, show that category on the card
    if let Some(slug) = &cat {
        qb.push("(c.slug = ").push_bind(slug.clone()).push(") DESC, ");
    }
    qb.push("c.name LIMIT 1) AS category_name
               FROM articles a
              WHERE a.status = 'published'");
    push_search_filters(&mut qb, &terms, &cat, &before);

    if terms.is_empty() {
        qb.push(" ORDER BY a.published_at DESC, a.id DESC");
    } else {
        // stories whose title contains the whole phrase come first, then newest
        qb.push(" ORDER BY CASE WHEN a.title LIKE ")
            .push_bind(like_pattern(&terms.join(" ")))
            .push(" THEN 0 ELSE 1 END, a.published_at DESC, a.id DESC");
    }
    qb.push(" LIMIT ").push_bind(limit).push(" OFFSET ").push_bind(offset);

    let mut items: Vec<SearchItem> = qb.build_query_as::<SearchItem>().fetch_all(pool).await?;
    for it in items.iter_mut() {
        if it.featured_image.starts_with('/') {
            it.featured_image = format!("{base}{}", it.featured_image);
        }
    }

    let mut cq = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM articles a WHERE a.status = 'published'");
    push_search_filters(&mut cq, &terms, &cat, &before);
    let total: i64 = cq.build_query_scalar().fetch_one(pool).await?;

    Ok(SearchPage { items, total, offset, limit })
}

// Categories for the department filter (only ones that have published stories)
pub async fn search_categories(pool: &MySqlPool) -> Result<Vec<SearchCategory>, sqlx::Error> {
    sqlx::query_as::<_, SearchCategory>(
        // if `category::public_by_slug` filters on a visibility column (e.g. is_active), add the same condition here
        "SELECT c.id, c.name, c.slug
           FROM categories c
          WHERE EXISTS (
                SELECT 1
                  FROM article_categories x
                  JOIN articles a ON a.id = x.article_id AND a.status = 'published'
                 WHERE x.category_id = c.id
          )
          ORDER BY c.name",
    )
    .fetch_all(pool)
    .await
}