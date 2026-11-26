//! What the public site's header and footer need, in one request each.
//! Also the editors' header highlights selection.

use std::collections::BTreeMap;
use chrono::NaiveDate;
use serde::Serialize;
use sqlx::MySqlPool;
use super::front_page::Brief;
use super::menu::{self, PublicItem, UrlPatterns};
use super::public_news::{self, Card};
use super::site_settings;

/// Stories in the header bar.
pub const HEADER_HIGHLIGHTS: usize = 3;

/// Where things live on the Vue site. `{slug}` and `{category}` are replaced.
pub struct Urls<'a> {
    pub category: &'a str,
    pub sub_category: &'a str,
    pub article: &'a str,
}

pub fn article_url(pattern: &str, slug: &str) -> String {
    pattern
        .replace("{id}/", "")
        .replace("{id}", "")
        .replace("%7Bid%7D/", "")
        .replace("%7Bid%7D", "")
        .replace("{slug}", slug)
        .trim_end_matches('/')
        .to_string()
}

// ======================= header =======================

#[derive(Debug, Serialize)]
pub struct HeaderSite {
    pub name: String,
    pub slogan: String,
    pub logo: String,
    /// optional picture of the portal name (used instead of the plain-text name)
    pub logo_wordmark: String,
    pub favicon: String,
}

#[derive(Debug, Serialize)]
pub struct HeaderCategory {
    pub id: i32,
    pub name: String,
    pub slug: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct Featured {
    #[serde(flatten)]
    pub card: Card,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct HeaderPayload {
    pub site: HeaderSite,
    pub menu: Vec<PublicItem>,
    pub categories: Vec<HeaderCategory>,
    pub featured: Vec<Featured>,
}

/// date = the past-day portal: the featured stories are then the newest ones published up to
/// the end of that day (the editors' current picks only apply to today's portal).
pub async fn header(pool: &MySqlPool, base: &str, urls: &Urls<'_>, date: Option<NaiveDate>) -> Result<HeaderPayload, sqlx::Error> {
    let settings = site_settings::public(pool, base).await?;
    let get = |k: &str| settings.get(k).cloned().unwrap_or_default();

    let patterns = UrlPatterns { category: urls.category, sub_category: urls.sub_category };
    let menu_items = menu::public_menus(pool, &patterns).await?.remove("header").unwrap_or_default();

    let cats: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT id, name, slug FROM categories WHERE is_active = 1 ORDER BY sort_order, name"
    )
    .fetch_all(pool)
    .await?;

    let categories = cats
        .into_iter()
        .map(|(id, name, slug)| HeaderCategory {
            id,
            url: urls.category.replace("{slug}", &slug),
            name,
            slug,
        })
        .collect();

    let cutoff = date.and_then(|d| d.succ_opt()).map(|d| d.format("%Y-%m-%d").to_string());

    let featured = public_news::header_featured(pool, base, HEADER_HIGHLIGHTS as i64, cutoff.as_deref())
        .await?
        .into_iter()
        .map(|card| Featured {
            url: article_url(urls.article, &card.slug),
            card,
        })
        .collect();

    Ok(HeaderPayload {
        site: HeaderSite {
            name: get("site_name"),
            slogan: get("site_slogan"),
            logo: get("logo"),
            logo_wordmark: get("logo_wordmark"),
            favicon: get("favicon"),
        },
        menu: menu_items,
        categories,
        featured,
    })
}

// ======================= footer =======================

const FOOTER_SITE_KEYS: [&str; 13] = [
    "site_name",
    "site_slogan",
    "site_description",
    "logo",
    "logo_wordmark",
    "publisher_name",
    "editor_name",
    "registration_number",
    "office_address",
    "contact_phone",
    "contact_phone_secondary",
    "contact_email",
    "copyright_text",
];

const FOOTER_EXTRA_KEYS: [&str; 3] = ["ads_email", "map_url", "favicon"];

const SOCIAL_KEYS: [(&str, &str); 5] = [
    ("facebook", "facebook_url"),
    ("twitter", "twitter_url"),
    ("youtube", "youtube_url"),
    ("instagram", "instagram_url"),
    ("linkedin", "linkedin_url"),
];

#[derive(Debug, Serialize)]
pub struct FooterPayload {
    pub site: BTreeMap<String, String>,
    pub social: BTreeMap<String, String>,
    /// footer_quick, footer_sections, footer_about
    pub menus: BTreeMap<String, Vec<PublicItem>>,
}

pub async fn footer(pool: &MySqlPool, base: &str, urls: &Urls<'_>) -> Result<FooterPayload, sqlx::Error> {
    let settings = site_settings::public(pool, base).await?;
    let pick = |keys: &[&str]| -> BTreeMap<String, String> {
        keys.iter().map(|k| (k.to_string(), settings.get(*k).cloned().unwrap_or_default())).collect()
    };

    let mut site = pick(&FOOTER_SITE_KEYS);
    site.extend(pick(&FOOTER_EXTRA_KEYS));

    let social = SOCIAL_KEYS
        .iter()
        .map(|(name, key)| (name.to_string(), settings.get(*key).cloned().unwrap_or_default()))
        .collect();

    let patterns = UrlPatterns { category: urls.category, sub_category: urls.sub_category };
    let menus = menu::public_menus(pool, &patterns)
        .await?
        .into_iter()
        .filter(|(location, _)| location.starts_with("footer_"))
        .collect();

    Ok(FooterPayload { site, social, menus })
}

// ======================= header highlights: the editors choice =======================

const BRIEF: &str = "a.id, a.title, a.featured_image AS thumb, COALESCE(DATE_FORMAT(a.published_at, '%Y-%m-%d %H:%i'), '') AS published_at";

pub async fn selected(pool: &MySqlPool) -> Result<Vec<Brief>, sqlx::Error> {
    sqlx::query_as::<_, Brief>(&format!(
        "SELECT {BRIEF} FROM header_highlights h JOIN articles a ON a.id = h.article_id WHERE a.status = 'published' ORDER BY h.position"
    ))
    .fetch_all(pool)
    .await
}

/// Published stories that are not chosen yet (newest first).
pub async fn pool(pool: &MySqlPool, limit: i64) -> Result<Vec<Brief>, sqlx::Error> {
    sqlx::query_as::<_, Brief>(&format!(
        "SELECT {BRIEF} FROM articles a WHERE a.status = 'published' AND a.id NOT IN (SELECT article_id FROM header_highlights) ORDER BY a.published_at DESC, a.id DESC LIMIT ?"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Replaces the selection. Ids must already be validated (published, unique, at most HEADER_HIGHLIGHTS).
pub async fn save(pool: &MySqlPool, ids: &[i32]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM header_highlights").execute(&mut *tx).await?;

    for (pos, id) in ids.iter().enumerate() {
        sqlx::query("INSERT INTO header_highlights (position, article_id) VALUES (?, ?)")
            .bind(pos as i32)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn article_urls() {
        assert_eq!(article_url("/news/{id}/{slug}", "election-day"), "/news/election-day");
        assert_eq!(article_url("/news/%7Bid%7D/{slug}", "election-day"), "/news/election-day");
        assert_eq!(article_url("/news/{slug}", ""), "/news", "no dangling slash when the slug is empty");
        assert_eq!(article_url("/খবর/{slug}", "নির্বাচন"), "/খবর/নির্বাচন");
    }

    #[test]
    fn past_day_cutoff_is_the_next_day() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(d.succ_opt().unwrap().format("%Y-%m-%d").to_string(), "2026-11-01");
    }
}
