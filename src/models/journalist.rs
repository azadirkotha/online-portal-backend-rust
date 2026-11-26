//! Journalist applications (public sign-up + admin review) and a journalist's own articles.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, MySql, MySqlPool, QueryBuilder, Transaction};
use ammonia::Builder;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::models::profile::{card_expired, clean_line, clean_multiline, valid_email, valid_phone, valid_upload_path, JOURNALIST_ROLE};

/// Status given to a journalist's new article (an admin publishes it).
/// Change it if your `articles.status` uses a different "not published yet" value.
pub const NEW_ARTICLE_STATUS: &str = "draft";
pub const ARTICLE_KINDS: [&str; 2] = ["news", "opinion"];

// ---------- small helpers ----------

/// ০-৯ -> 0-9
pub fn ascii_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{09E6}'..='\u{09EF}' => char::from_digit(c as u32 - 0x09E6, 10).unwrap_or(c),
            _ => c,
        })
        .collect()
}

// ---------- public verification ----------

#[derive(Debug, FromRow)]
struct VerifyRow {
    name: String,
    photo: String,
    designation: String,
    journalist_number: String,
    card_issue_date: String,
    card_expiry_date: String,
}

/// Only these fields are ever public: no phone, email, NID or blood group.
#[derive(Debug, Serialize)]
pub struct VerifiedJournalist {
    pub name: String,
    pub photo: String,
    pub designation: String,
    pub journalist_number: String,
    pub card_issue_date: String,
    pub card_expiry_date: String,
    /// "valid" or "expired"
    pub status: &'static str,
}

/// Exact match on the press card number of an active journalist.
pub async fn verify_journalist(pool: &MySqlPool, base: &str, number: &str) -> Result<Option<VerifiedJournalist>, sqlx::Error> {
    let ascii = ascii_digits(number); // the number may have been typed with Bengali digits
    let row = sqlx::query_as::<_, VerifyRow>(
        "SELECT u.name,
                COALESCE(u.photo, '') AS photo,
                COALESCE(u.designation, '') AS designation,
                COALESCE(u.journalist_number, '') AS journalist_number,
                COALESCE(DATE_FORMAT(u.card_issue_date, '%Y-%m-%d'), '') AS card_issue_date,
                COALESCE(DATE_FORMAT(u.card_expiry_date, '%Y-%m-%d'), '') AS card_expiry_date
           FROM users u
          WHERE u.journalist_number IN (?, ?)
            AND u.status = 1
            AND EXISTS (
                SELECT 1 FROM user_roles ur JOIN roles r ON r.id = ur.role_id
                 WHERE ur.user_id = u.id AND r.name = ?
            )
          LIMIT 1",
    )
    .bind(number)
    .bind(&ascii)
    .bind(JOURNALIST_ROLE)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let status = if !r.card_expiry_date.is_empty() && card_expired(&r.card_expiry_date) { "expired" } else { "valid" };
        let photo = if r.photo.starts_with('/') { format!("{base}{}", r.photo) } else { r.photo };
        VerifiedJournalist {
            name: r.name,
            photo,
            designation: r.designation,
            journalist_number: r.journalist_number,
            card_issue_date: r.card_issue_date,
            card_expiry_date: r.card_expiry_date,
            status,
        }
    }))
}

/// Bangladesh NID: 10, 13 or 17 digits.
pub fn valid_nid(n: &str) -> bool {
    matches!(n.len(), 10 | 13 | 17) && n.bytes().all(|b| b.is_ascii_digit())
}

pub fn valid_username(u: &str) -> bool {
    (3..=30).contains(&u.len()) && u.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

#[test]
fn html_to_text_strips_tags() {
    assert_eq!(html_to_text("<p>প্রথম <b>অনুচ্ছেদ</b></p><p>Second &amp; last</p>"), "প্রথম অনুচ্ছেদ\n\nSecond & last");
}

/// Cleans the editor's HTML. Only the tags below survive; scripts, event handlers, styles
/// (except text-align) and images from outside our own uploads are removed.
/// Image paths are stored as /uploads/... so they stay valid if the domain changes.
/// Cleans the editor's HTML. Only the tags below survive; scripts, event handlers, styles
/// (except text-align, and a percent width on images) and images from outside our own uploads are removed.
/// Image paths are stored as /uploads/... so they stay valid if the domain changes.
pub fn sanitize_body(html: &str, base: &str) -> String {
    let tags: HashSet<&str> = ["p", "br", "h2", "h3", "h4", "strong", "b", "em", "i", "u", "s", "blockquote", "ul", "ol", "li", "a", "img"]
        .into_iter()
        .collect();

    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attrs.insert("a", ["href"].into_iter().collect());
    tag_attrs.insert("img", ["src", "alt", "style"].into_iter().collect());
    for t in ["p", "h2", "h3", "h4", "blockquote", "li"] {
        tag_attrs.insert(t, ["style"].into_iter().collect());
    }

    let schemes: HashSet<&str> = ["http", "https", "mailto"].into_iter().collect();
    let abs_prefix = format!("{}/uploads/", base.trim_end_matches('/'));

    Builder::default()
        .tags(tags)
        .tag_attributes(tag_attrs)
        .generic_attributes(HashSet::new())
        .url_schemes(schemes)
        .link_rel(Some("noopener noreferrer nofollow"))
        .attribute_filter(move |element, attribute, value| match (element, attribute) {
            ("img", "src") => {
                let path = match value.strip_prefix(abs_prefix.as_str()) {
                    Some(rest) => format!("/uploads/{rest}"),
                    None => value.to_string(),
                };
                if valid_upload_path(&path) { Some(Cow::Owned(path)) } else { None }
            }
            // image size: only "width: NN%" with NN between 5 and 100
            ("img", "style") => {
                let v: String = value.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
                let v = v.trim_end_matches(';');
                let pct = v
                    .strip_prefix("width:")
                    .and_then(|r| r.strip_suffix('%'))
                    .and_then(|n| n.parse::<f32>().ok());
                match pct {
                    Some(p) if (5.0..=100.0).contains(&p) => Some(Cow::Owned(format!("width: {}%", p.round() as u32))),
                    _ => None,
                }
            }
            // every other element: the only style allowed is text alignment
            (_, "style") => {
                let v: String = value.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
                let v = v.trim_end_matches(';');
                match v {
                    "text-align:left" | "text-align:center" | "text-align:right" | "text-align:justify" => {
                        Some(Cow::Owned(format!("text-align: {}", &v["text-align:".len()..])))
                    }
                    _ => None,
                }
            }
            _ => Some(Cow::Borrowed(value)),
        })
        .clean(html)
        .to_string()
}

/// Back to editable text (used when a journalist reopens a draft).
pub fn html_to_text(html: &str) -> String {
    let s = html
        .replace("</p>", "\n\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// URL slug that keeps Bengali letters (including the hasanta / vowel signs).
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = true;
    for c in title.chars() {
        if c.is_alphanumeric() || ('\u{0980}'..='\u{09FF}').contains(&c) {
            out.extend(c.to_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let out: String = out.trim_matches('-').chars().take(120).collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "news".to_string() } else { out }
}

// ---------- application form ----------

#[derive(Debug, Default, Clone)]
pub struct ApplyFields {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub username: String,
    pub password: String,
    pub nid_number: String,
    pub occupation: String,
    pub current_platform: String,
    pub platform_phone: String,
    pub about: String,
}

impl ApplyFields {
    pub fn clean(&self) -> Result<ApplyFields, &'static str> {
        let name = clean_line(&self.name);
        if !(2..=100).contains(&name.chars().count()) {
            return Err("নাম ২ থেকে ১০০ অক্ষরের মধ্যে হতে হবে।");
        }
        let email = self.email.trim().to_lowercase();
        if !valid_email(&email) {
            return Err("সঠিক ইমেইল ঠিকানা দিন।");
        }
        let phone = clean_line(&self.phone);
        if !valid_phone(&phone) {
            return Err("সঠিক মোবাইল নম্বর দিন।");
        }
        let username = self.username.trim().to_string();
        if !valid_username(&username) {
            return Err("ইউজারনেম ৩–৩০ অক্ষরের হতে হবে (ইংরেজি অক্ষর, সংখ্যা, _ . -)।");
        }
        if !(8..=128).contains(&self.password.chars().count()) {
            return Err("পাসওয়ার্ড কমপক্ষে ৮ অক্ষরের হতে হবে।");
        }
        let nid_number = ascii_digits(&self.nid_number.split_whitespace().collect::<String>());
        if !valid_nid(&nid_number) {
            return Err("সঠিক NID নম্বর দিন (১০, ১৩ বা ১৭ সংখ্যা)।");
        }
        let occupation = clean_line(&self.occupation);
        if !(2..=100).contains(&occupation.chars().count()) {
            return Err("আপনার পেশা লিখুন।");
        }
        let current_platform = clean_line(&self.current_platform);
        if current_platform.chars().count() > 150 {
            return Err("বর্তমান প্রতিষ্ঠানের নাম অনেক বড়।");
        }
        let platform_phone = clean_line(&self.platform_phone);
        if !platform_phone.is_empty() && !valid_phone(&platform_phone) {
            return Err("প্রতিষ্ঠানের ফোন নম্বর সঠিক নয়।");
        }
        let about = clean_multiline(&self.about);
        if !(20..=1000).contains(&about.chars().count()) {
            return Err("নিজের সম্পর্কে ২০ থেকে ১০০০ অক্ষরের মধ্যে লিখুন।");
        }
        Ok(ApplyFields {
            name,
            email,
            phone,
            username,
            password: self.password.clone(),
            nid_number,
            occupation,
            current_platform,
            platform_phone,
            about,
        })
    }
}

pub enum ApplyError {
    Duplicate,
    Db(sqlx::Error),
}

fn classify(e: sqlx::Error) -> ApplyError {
    if e.as_database_error().map(|d| d.is_unique_violation()).unwrap_or(false) {
        ApplyError::Duplicate
    } else {
        ApplyError::Db(e)
    }
}

pub async fn nid_taken(pool: &MySqlPool, nid: &str) -> Result<bool, sqlx::Error> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journalist_applications WHERE nid_number = ? AND status IN ('pending', 'approved')",
    )
    .bind(nid)
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

/// Creates the (inactive) user and its application in one transaction.
pub async fn create_application(
    pool: &MySqlPool,
    f: &ApplyFields,
    password_hash: &str,
    photo_file: &str,
    nid_file: &str,
    resume_file: &str,
) -> Result<i32, ApplyError> {
    let mut tx = pool.begin().await.map_err(ApplyError::Db)?;

    // same as /auth/register: no roles, status stays at its default (pending)
    let user = sqlx::query("INSERT INTO users (name, email, phone, username, password) VALUES (?, ?, ?, ?, ?)")
        .bind(&f.name)
        .bind(&f.email)
        .bind(&f.phone)
        .bind(&f.username)
        .bind(password_hash)
        .execute(&mut *tx)
        .await
        .map_err(classify)?;
    let user_id = user.last_insert_id() as i32;

    sqlx::query(
        "INSERT INTO journalist_applications
            (user_id, full_name, nid_number, phone, occupation, current_platform, platform_phone, about, photo_file, nid_file, resume_file)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(&f.name)
    .bind(&f.nid_number)
    .bind(&f.phone)
    .bind(&f.occupation)
    .bind(&f.current_platform)
    .bind(&f.platform_phone)
    .bind(&f.about)
    .bind(photo_file)
    .bind(nid_file)
    .bind(resume_file)
    .execute(&mut *tx)
    .await
    .map_err(classify)?;

    tx.commit().await.map_err(ApplyError::Db)?;
    Ok(user_id)
}

pub async fn application_status(pool: &MySqlPool, user_id: i32) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM journalist_applications WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

pub async fn is_approved_journalist(pool: &MySqlPool, user_id: i32) -> Result<bool, sqlx::Error> {
    Ok(application_status(pool, user_id).await?.as_deref() == Some("approved"))
}

// ---------- admin review ----------

#[derive(Debug, Serialize, FromRow)]
pub struct ApplicationRow {
    pub id: i32,
    pub user_id: i32,
    pub full_name: String,
    pub username: String,
    pub email: String,
    pub phone: String,
    pub occupation: String,
    pub current_platform: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct ApplicationDetail {
    pub id: i32,
    pub user_id: i32,
    pub full_name: String,
    pub username: String,
    pub email: String,
    pub phone: String,
    pub nid_number: String,
    pub occupation: String,
    pub current_platform: String,
    pub platform_phone: String,
    pub about: String,
    pub status: String,
    pub review_note: String,
    pub created_at: String,
    pub reviewed_at: String,
}

#[derive(Debug, Serialize)]
pub struct ApplicationPage {
    pub items: Vec<ApplicationRow>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
}

pub async fn applications(pool: &MySqlPool, status: &str, page: i64, per_page: i64) -> Result<ApplicationPage, sqlx::Error> {
    let filter = status != "all";

    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT a.id, a.user_id, a.full_name, u.username, u.email, a.phone, a.occupation, a.current_platform, a.status,
                DATE_FORMAT(a.created_at, '%Y-%m-%dT%H:%i:%s') AS created_at
           FROM journalist_applications a JOIN users u ON u.id = a.user_id
          WHERE 1 = 1",
    );
    if filter {
        qb.push(" AND a.status = ").push_bind(status.to_string());
    }
    qb.push(" ORDER BY a.id DESC LIMIT ").push_bind(per_page).push(" OFFSET ").push_bind((page - 1) * per_page);
    let items = qb.build_query_as::<ApplicationRow>().fetch_all(pool).await?;

    let mut cq = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM journalist_applications a WHERE 1 = 1");
    if filter {
        cq.push(" AND a.status = ").push_bind(status.to_string());
    }
    let total: i64 = cq.build_query_scalar().fetch_one(pool).await?;

    Ok(ApplicationPage { items, total, page, per_page, total_pages: ((total + per_page - 1) / per_page).max(1) })
}

pub async fn application(pool: &MySqlPool, id: i32) -> Result<Option<ApplicationDetail>, sqlx::Error> {
    sqlx::query_as::<_, ApplicationDetail>(
        "SELECT a.id, a.user_id, a.full_name, u.username, u.email, a.phone, a.nid_number, a.occupation,
                a.current_platform, a.platform_phone, a.about, a.status, a.review_note,
                DATE_FORMAT(a.created_at, '%Y-%m-%dT%H:%i:%s') AS created_at,
                COALESCE(DATE_FORMAT(a.reviewed_at, '%Y-%m-%dT%H:%i:%s'), '') AS reviewed_at
           FROM journalist_applications a JOIN users u ON u.id = a.user_id
          WHERE a.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// File name (inside the private folder) of one document: kind = photo | nid | resume
pub async fn application_file(pool: &MySqlPool, id: i32, kind: &str) -> Result<Option<String>, sqlx::Error> {
    let sql = match kind {
        "photo" => "SELECT photo_file FROM journalist_applications WHERE id = ?",
        "nid" => "SELECT nid_file FROM journalist_applications WHERE id = ?",
        "resume" => "SELECT resume_file FROM journalist_applications WHERE id = ?",
        _ => return Ok(None),
    };
    sqlx::query_scalar(sql).bind(id).fetch_optional(pool).await
}

pub enum ReviewOutcome {
    Done(i32), // user id
    NotFound,
    NotPending,
    RoleMissing,
}

/// Approve: application -> approved, user activated, "Journalist" role assigned.
pub async fn approve(pool: &MySqlPool, id: i32, reviewer: i32) -> Result<ReviewOutcome, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let row: Option<(i32, String)> =
        sqlx::query_as("SELECT user_id, status FROM journalist_applications WHERE id = ? FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((user_id, status)) = row else { return Ok(ReviewOutcome::NotFound) };
    if status != "pending" {
        return Ok(ReviewOutcome::NotPending);
    }

    let roles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE name = ?")
        .bind(JOURNALIST_ROLE)
        .fetch_one(&mut *tx)
        .await?;
    if roles == 0 {
        return Ok(ReviewOutcome::RoleMissing);
    }

    sqlx::query("UPDATE journalist_applications SET status = 'approved', reviewed_by = ?, reviewed_at = NOW() WHERE id = ?")
        .bind(reviewer)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET status = 1 WHERE id = ?").bind(user_id).execute(&mut *tx).await?;
    // assumes tables roles(id, name) and user_roles(user_id, role_id)
    sqlx::query("INSERT IGNORE INTO user_roles (user_id, role_id) SELECT ?, r.id FROM roles r WHERE r.name = ?")
        .bind(user_id)
        .bind(JOURNALIST_ROLE)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(ReviewOutcome::Done(user_id))
}

/// Returns false when the application does not exist or was already reviewed.
pub async fn reject(pool: &MySqlPool, id: i32, reviewer: i32, note: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE journalist_applications SET status = 'rejected', review_note = ?, reviewed_by = ?, reviewed_at = NOW()
          WHERE id = ? AND status = 'pending'",
    )
    .bind(note)
    .bind(reviewer)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn set_user_photo(pool: &MySqlPool, user_id: i32, path: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET photo = ? WHERE id = ?").bind(path).bind(user_id).execute(pool).await.map(|_| ())
}

// ---------- password ----------

/// The stored password hash of a user.
pub async fn password_hash(pool: &MySqlPool, user_id: i32) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT password FROM users WHERE id = ?").bind(user_id).fetch_optional(pool).await
}

pub async fn set_password(pool: &MySqlPool, user_id: i32, hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password = ? WHERE id = ?").bind(hash).bind(user_id).execute(pool).await.map(|_| ())
}

// ---------- a journalist's own articles ----------

pub struct ArticleClean {
    pub title: String,
    pub summary: String,
    pub body_html: String,
    pub kind: String,
    pub category_ids: Vec<i32>,
    pub featured_image: String,
    pub caption: String,
    pub news_source: String,
}

#[derive(Debug, Deserialize)]
pub struct ArticleInput {
    pub title: String,
    pub summary: String,
    pub body: String, // HTML from the rich text editor
    pub kind: String,
    pub category_ids: Vec<i32>,
    pub featured_image: Option<String>,
    pub featured_image_caption: Option<String>,
    pub news_source: Option<String>,
}

impl ArticleInput {
    pub fn clean(&self, base: &str) -> Result<ArticleClean, &'static str> {
        let title = clean_line(&self.title);
        if !(5..=200).contains(&title.chars().count()) {
            return Err("শিরোনাম ৫ থেকে ২০০ অক্ষরের মধ্যে হতে হবে।");
        }
        let summary = clean_line(&self.summary);
        if !(10..=300).contains(&summary.chars().count()) {
            return Err("সারসংক্ষেপ ১০ থেকে ৩০০ অক্ষরের মধ্যে হতে হবে।");
        }
        if self.body.len() > 400_000 {
            return Err("লেখাটি অনেক বড়।");
        }
        let body_html = sanitize_body(&self.body, base);
        let text_len = html_to_text(&body_html).chars().count();
        if !(50..=30000).contains(&text_len) {
            return Err("লেখার মূল অংশ ৫০ থেকে ৩০,০০০ অক্ষরের মধ্যে হতে হবে।");
        }
        let kind = self.kind.trim().to_string();
        if !ARTICLE_KINDS.contains(&kind.as_str()) {
            return Err("লেখার ধরন সঠিক নয়।");
        }
        let mut ids = self.category_ids.clone();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() || ids.len() > 5 {
            return Err("১ থেকে ৫টি বিভাগ বেছে নিন।");
        }
        let featured_image = self.featured_image.clone().unwrap_or_default().trim().to_string();
        if !featured_image.is_empty() && !valid_upload_path(&featured_image) {
            return Err("ছবিটি সঠিকভাবে আপলোড করা হয়নি।");
        }
        let caption = clean_line(&self.featured_image_caption.clone().unwrap_or_default());
        if caption.chars().count() > 255 {
            return Err("ছবির ক্যাপশন অনেক বড় (সর্বোচ্চ ২৫৫ অক্ষর)।");
        }
        let news_source = clean_line(&self.news_source.clone().unwrap_or_default());
        if news_source.is_empty() || news_source.chars().count() > 150 {
            return Err("সংবাদ সূত্র দিন (সর্বোচ্চ ১৫০ অক্ষর)।");
        }
        Ok(ArticleClean { title, summary, body_html, kind, category_ids: ids, featured_image, caption, news_source })
    }
}

#[derive(Debug, Serialize, FromRow)]
pub struct CategoryOption {
    pub id: i32,
    pub name: String,
    pub slug: String,
}

pub async fn all_categories(pool: &MySqlPool) -> Result<Vec<CategoryOption>, sqlx::Error> {
    sqlx::query_as::<_, CategoryOption>("SELECT id, name, slug FROM categories ORDER BY name").fetch_all(pool).await
}

async fn unique_slug(pool: &MySqlPool, base: &str) -> Result<String, sqlx::Error> {
    for n in 1..200 {
        let candidate = if n == 1 { base.to_string() } else { format!("{base}-{n}") };
        let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM articles WHERE slug = ?").bind(&candidate).fetch_one(pool).await?;
        if taken == 0 {
            return Ok(candidate);
        }
    }
    Ok(format!("{base}-{}", chrono::Utc::now().timestamp()))
}

/// Replaces the article's categories. Returns how many valid categories were linked.
async fn set_categories(tx: &mut Transaction<'_, MySql>, article_id: i32, ids: &[i32]) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM article_categories WHERE article_id = ?").bind(article_id).execute(&mut **tx).await?;
    let mut linked = 0;
    for id in ids {
        let r = sqlx::query("INSERT INTO article_categories (article_id, category_id) SELECT ?, c.id FROM categories c WHERE c.id = ?")
            .bind(article_id)
            .bind(id)
            .execute(&mut **tx)
            .await?;
        linked += r.rows_affected();
    }
    Ok(linked)
}

/// None = none of the chosen categories exist.
pub async fn create_article(pool: &MySqlPool, author: i32, a: &ArticleClean) -> Result<Option<(i32, String)>, sqlx::Error> {
    let slug = unique_slug(pool, &slugify(&a.title)).await?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query(
        "INSERT INTO articles (title, slug, summary, body, kind, featured_image, featured_image_caption, news_source, author_id, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&a.title)
    .bind(&slug)
    .bind(&a.summary)
    .bind(&a.body_html)
    .bind(&a.kind)
    .bind(&a.featured_image)
    .bind(&a.caption)
    .bind(&a.news_source)
    .bind(author)
    .bind(NEW_ARTICLE_STATUS)
    .execute(&mut *tx)
    .await?;
    let id = r.last_insert_id() as i32;
    if set_categories(&mut tx, id, &a.category_ids).await? == 0 {
        return Ok(None); // dropping the transaction rolls it back
    }
    tx.commit().await?;
    Ok(Some((id, slug)))
}

/// Status of the journalist's own article (None = not theirs / not found).
pub async fn own_article_status(pool: &MySqlPool, id: i32, author: i32) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT CAST(status AS CHAR) FROM articles WHERE id = ? AND author_id = ?")
        .bind(id)
        .bind(author)
        .fetch_optional(pool)
        .await
}

/// false = none of the chosen categories exist.
pub async fn update_article(pool: &MySqlPool, id: i32, author: i32, a: &ArticleClean) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE articles SET title = ?, summary = ?, body = ?, kind = ?, featured_image = ?, featured_image_caption = ?, news_source = ?
          WHERE id = ? AND author_id = ? AND CAST(status AS CHAR) <> 'published'",
    )
    .bind(&a.title)
    .bind(&a.summary)
    .bind(&a.body_html)
    .bind(&a.kind)
    .bind(&a.featured_image)
    .bind(&a.caption)
    .bind(&a.news_source)
    .bind(id)
    .bind(author)
    .execute(&mut *tx)
    .await?;
    if set_categories(&mut tx, id, &a.category_ids).await? == 0 {
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}

pub async fn delete_article(pool: &MySqlPool, id: i32, author: i32) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for sql in [
        "DELETE FROM article_categories WHERE article_id = ?",
        "DELETE FROM article_sub_categories WHERE article_id = ?",
        "DELETE FROM article_tags WHERE article_id = ?",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM articles WHERE id = ? AND author_id = ? AND CAST(status AS CHAR) <> 'published'")
        .bind(id)
        .bind(author)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

#[derive(Debug, Serialize, FromRow)]
pub struct OwnArticleRow {
    pub id: i32,
    pub title: String,
    pub slug: String,
    pub status: String,
    pub kind: String,
    pub published_at: String,
    pub view_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ArticleSummary {
    pub total: i64,
    pub published: i64,
    pub drafts: i64,
    pub views: i64,
}

#[derive(Debug, Serialize)]
pub struct OwnArticlePage {
    pub items: Vec<OwnArticleRow>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub summary: ArticleSummary,
}

pub async fn own_articles(pool: &MySqlPool, author: i32, status: Option<&str>, page: i64, per_page: i64) -> Result<OwnArticlePage, sqlx::Error> {
    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT a.id, a.title, a.slug, CAST(a.status AS CHAR) AS status, CAST(a.kind AS CHAR) AS kind,
                COALESCE(DATE_FORMAT(a.published_at, '%Y-%m-%dT%H:%i:%s'), '') AS published_at,
                CAST(COALESCE(a.view_count, 0) AS SIGNED) AS view_count
           FROM articles a WHERE a.author_id = ",
    );
    qb.push_bind(author);
    if let Some(s) = status {
        qb.push(" AND CAST(a.status AS CHAR) = ").push_bind(s.to_string());
    }
    qb.push(" ORDER BY a.id DESC LIMIT ").push_bind(per_page).push(" OFFSET ").push_bind((page - 1) * per_page);
    let items = qb.build_query_as::<OwnArticleRow>().fetch_all(pool).await?;

    let mut cq = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM articles a WHERE a.author_id = ");
    cq.push_bind(author);
    if let Some(s) = status {
        cq.push(" AND CAST(a.status AS CHAR) = ").push_bind(s.to_string());
    }
    let total: i64 = cq.build_query_scalar().fetch_one(pool).await?;

    let (all, published, views): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),
                CAST(COALESCE(SUM(CASE WHEN CAST(status AS CHAR) = 'published' THEN 1 ELSE 0 END), 0) AS SIGNED),
                CAST(COALESCE(SUM(view_count), 0) AS SIGNED)
           FROM articles WHERE author_id = ?",
    )
    .bind(author)
    .fetch_one(pool)
    .await?;

    Ok(OwnArticlePage {
        items,
        total,
        page,
        per_page,
        total_pages: ((total + per_page - 1) / per_page).max(1),
        summary: ArticleSummary { total: all, published, drafts: all - published, views },
    })
}

#[derive(Debug, FromRow)]
pub struct OwnArticleFull {
    pub id: i32,
    pub title: String,
    pub slug: String,
    pub summary: String,
    pub body: String,
    pub kind: String,
    pub status: String,
    pub featured_image: String,
    pub featured_image_caption: String,
    pub news_source: String,
}

pub async fn own_article(pool: &MySqlPool, id: i32, author: i32) -> Result<Option<(OwnArticleFull, Vec<i32>)>, sqlx::Error> {
    let row = sqlx::query_as::<_, OwnArticleFull>(
        "SELECT a.id, a.title, a.slug, COALESCE(a.summary, '') AS summary, COALESCE(a.body, '') AS body,
                CAST(a.kind AS CHAR) AS kind, CAST(a.status AS CHAR) AS status,
                COALESCE(a.featured_image, '') AS featured_image,
                COALESCE(a.featured_image_caption, '') AS featured_image_caption,
                COALESCE(a.news_source, '') AS news_source
           FROM articles a WHERE a.id = ? AND a.author_id = ?",
    )
    .bind(id)
    .bind(author)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let cats: Vec<i32> = sqlx::query_scalar("SELECT category_id FROM article_categories WHERE article_id = ?")
        .bind(id)
        .fetch_all(pool)
        .await?;
    Ok(Some((row, cats)))
}

// ---------- the journalist's own (read-only) info ----------

#[derive(Debug, Serialize, FromRow)]
pub struct MyInfo {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub username: String,
    pub photo: String,
    pub bio: String,
    pub designation: String,
    pub blood_group: String,
    pub occupation: String,
    pub current_platform: String,
    pub journalist_number: String,
    pub card_issue_date: String,
    pub card_expiry_date: String,
    /// "valid", "expired" or "" when there is no expiry date
    #[sqlx(skip)]
    pub card_status: String,
}

pub async fn my_info(pool: &MySqlPool, base: &str, user_id: i32) -> Result<Option<MyInfo>, sqlx::Error> {
    let row = sqlx::query_as::<_, MyInfo>(
        "SELECT u.name,
                COALESCE(u.email, '') AS email,
                COALESCE(u.phone, '') AS phone,
                u.username,
                COALESCE(u.photo, '') AS photo,
                COALESCE(u.bio, '') AS bio,
                COALESCE(u.designation, '') AS designation,
                COALESCE(u.blood_group, '') AS blood_group,
                COALESCE(a.occupation, '') AS occupation,
                COALESCE(a.current_platform, '') AS current_platform,
                COALESCE(u.journalist_number, '') AS journalist_number,
                COALESCE(DATE_FORMAT(u.card_issue_date, '%Y-%m-%d'), '') AS card_issue_date,
                COALESCE(DATE_FORMAT(u.card_expiry_date, '%Y-%m-%d'), '') AS card_expiry_date
           FROM users u
           LEFT JOIN journalist_applications a ON a.user_id = u.id
          WHERE u.id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|mut r| {
        if r.photo.starts_with('/') {
            r.photo = format!("{base}{}", r.photo);
        }
        r.card_status = if r.card_expiry_date.is_empty() {
            String::new()
        } else if card_expired(&r.card_expiry_date) {
            "expired".to_string()
        } else {
            "valid".to_string()
        };
        r
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nid_numbers() {
        assert!(valid_nid("1234567890"));
        assert!(valid_nid("1234567890123"));
        assert!(valid_nid("12345678901234567"));
        for bad in ["", "12345", "12345678901", "12345abcde", "১২৩৪৫৬৭৮৯০"] {
            assert!(!valid_nid(bad), "{bad:?}");
        }
        assert_eq!(ascii_digits("১২৩৪৫৬৭৮৯০"), "1234567890");
    }

    #[test]
    fn usernames() {
        assert!(valid_username("reporter_01"));
        for bad in ["ab", "has space", "ব্যবহারকারী", &"x".repeat(31)] {
            assert!(!valid_username(bad), "{bad:?}");
        }
    }

    #[test]
    fn slugs_keep_bengali_conjuncts() {
        assert_eq!(slugify("নির্বাচন ২০২৫: জনগণের কণ্ঠস্বর!"), "নির্বাচন-২০২৫-জনগণের-কণ্ঠস্বর");
        assert_eq!(slugify("Hello,  World!"), "hello-world");
        assert_eq!(slugify("!!!"), "news");
    }

    #[test]
    fn text_html_round_trip() {
        let text = "প্রথম অনুচ্ছেদ <b>\nদ্বিতীয় লাইন\n\nSecond & last";
        let html = text_to_html(text);
        assert_eq!(html, "<p>প্রথম অনুচ্ছেদ &lt;b&gt;<br>দ্বিতীয় লাইন</p><p>Second &amp; last</p>");
        assert_eq!(html_to_text(&html), text);
    }

    #[test]
    fn sanitizer_keeps_formatting_and_removes_danger() {
        let base = "http://127.0.0.1:3000";
        let dirty = r#"<p style="text-align: center" onclick="x()">Hi <script>alert(1)</script><b>there</b></p><img src="http://evil.com/a.png"><img src="http://127.0.0.1:3000/uploads/2026/11/ab.jpg" onerror="x()"><a href="javascript:alert(1)">bad</a><a href="https://ok.com">ok</a><h2 style="color:red">T</h2>"#;
        let clean = sanitize_body(dirty, base);
        assert!(clean.contains("text-align: center"));
        assert!(!clean.contains("script") && !clean.contains("onclick") && !clean.contains("onerror"));
        assert!(!clean.contains("evil.com"));
        assert!(clean.contains(r#"src="/uploads/2026/11/ab.jpg""#));
        assert!(!clean.contains("javascript:"));
        assert!(clean.contains("https://ok.com"));
        assert!(!clean.contains("color:red"));
    }

    #[test]
    fn sanitizer_allows_only_percent_width_on_images() {
        let ok = sanitize_body(r#"<img src="/uploads/2026/11/a.jpg" style="width: 55%">"#, "http://x");
        assert!(ok.contains("width: 55%"));
        for bad in ["width: 500%", "position: fixed", "width: 55%; position: fixed", "width: 50px"] {
            let html = format!(r#"<img src="/uploads/2026/11/a.jpg" style="{bad}">"#);
            let out = sanitize_body(&html, "http://x");
            assert!(!out.contains("style"), "{bad}: {out}");
        }
    }
}