use std::collections::{BTreeMap, HashMap};

use sqlx::MySqlPool;

use super::profile::{clean_line, clean_multiline, valid_email, valid_http_url, valid_phone, valid_upload_path};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Textarea,
    Email,
    Url,
    Phone,
    Image,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Textarea => "textarea",
            Kind::Email => "email",
            Kind::Url => "url",
            Kind::Phone => "phone",
            Kind::Image => "image",
        }
    }
    /// value of the HTML `type` attribute for single-line inputs
    pub fn input_type(self) -> &'static str {
        match self {
            Kind::Email => "email",
            Kind::Url => "url",
            Kind::Phone => "tel",
            _ => "text",
        }
    }
}

pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub kind: Kind,
    pub max: usize,
    pub help: &'static str,
}

/// (key, title, hint)
pub const GROUPS: [(&str, &str, &str); 6] = [
    ("general", "General", "How the portal presents itself."),
    ("publisher", "Publisher and editor", "Shown in the footer and on the about / contact pages."),
    ("contact", "Contact and office", "Public contact details."),
    ("social", "Social media", "Full links, starting with https://"),
    ("footer", "Footer", "Text at the very bottom of every page."),
    ("seo", "Search and sharing", "Used by search engines and when a link is shared."),
];

/// Every setting the portal knows. To add one, add a line here: the form, validation and
/// the public API pick it up automatically.
pub const FIELDS: [Field; 25] = [
    Field { key: "site_name", label: "Portal name", group: "general", kind: Kind::Text, max: 120, help: "" },
    Field { key: "site_slogan", label: "Slogan", group: "general", kind: Kind::Text, max: 200, help: "A short line shown next to the logo." },
    Field { key: "site_description", label: "About the portal", group: "general", kind: Kind::Textarea, max: 600, help: "A few sentences for the footer and the about page." },
    Field { key: "logo", label: "Logo", group: "general", kind: Kind::Image, max: 500, help: "PNG or WebP, ideally with a transparent background." },
    Field { key: "logo_wordmark", label: "Name as an image (wordmark)", group: "general", kind: Kind::Image, max: 500, help: "Optional. If set, the header shows this picture next to the logo instead of the name text." },
    Field { key: "favicon", label: "Favicon", group: "general", kind: Kind::Image, max: 500, help: "A square PNG, at least 64 x 64 pixels." },

    Field { key: "publisher_name", label: "Publisher", group: "publisher", kind: Kind::Text, max: 120, help: "" },
    Field { key: "editor_name", label: "Editor", group: "publisher", kind: Kind::Text, max: 120, help: "" },
    Field { key: "registration_number", label: "Registration number", group: "publisher", kind: Kind::Text, max: 80, help: "Newspaper / online media registration." },

    Field { key: "office_address", label: "Office address", group: "contact", kind: Kind::Textarea, max: 400, help: "" },
    Field { key: "contact_phone", label: "Phone", group: "contact", kind: Kind::Phone, max: 40, help: "" },
    Field { key: "contact_phone_secondary", label: "Second phone", group: "contact", kind: Kind::Phone, max: 40, help: "" },
    Field { key: "contact_email", label: "Email", group: "contact", kind: Kind::Email, max: 255, help: "" },
    Field { key: "ads_email", label: "Advertising email", group: "contact", kind: Kind::Email, max: 255, help: "" },
    Field { key: "map_url", label: "Map link", group: "contact", kind: Kind::Url, max: 500, help: "Link to the office on a map." },

    Field { key: "facebook_url", label: "Facebook", group: "social", kind: Kind::Url, max: 500, help: "" },
    Field { key: "twitter_url", label: "X (Twitter)", group: "social", kind: Kind::Url, max: 500, help: "" },
    Field { key: "youtube_url", label: "YouTube", group: "social", kind: Kind::Url, max: 500, help: "" },
    Field { key: "instagram_url", label: "Instagram", group: "social", kind: Kind::Url, max: 500, help: "" },
    Field { key: "linkedin_url", label: "LinkedIn", group: "social", kind: Kind::Url, max: 500, help: "" },

    Field { key: "copyright_text", label: "Copyright line", group: "footer", kind: Kind::Text, max: 200, help: "For example: © 2026 Your Portal. All rights reserved." },

    Field { key: "meta_title", label: "Default page title", group: "seo", kind: Kind::Text, max: 120, help: "Used when a page has no title of its own." },
    Field { key: "meta_description", label: "Default description", group: "seo", kind: Kind::Textarea, max: 300, help: "Shown in search results." },
    Field { key: "meta_keywords", label: "Keywords", group: "seo", kind: Kind::Text, max: 300, help: "Comma separated." },
    Field { key: "share_image", label: "Default share image", group: "seo", kind: Kind::Image, max: 500, help: "Shown when a link to the portal is shared. 1200 x 630 works best." },
];

/// Cleans and checks one value. Everything is optional, so an empty value is always fine.
pub fn clean(field: &Field, raw: &str) -> Result<String, &'static str> {
    let value = match field.kind {
        Kind::Textarea => clean_multiline(raw),
        _ => clean_line(raw),
    };
    if value.is_empty() {
        return Ok(value);
    }
    if value.chars().count() > field.max {
        return Err("is too long");
    }
    let ok = match field.kind {
        Kind::Text | Kind::Textarea => true,
        Kind::Email => valid_email(&value),
        Kind::Phone => valid_phone(&value),
        Kind::Url => valid_http_url(&value),
        Kind::Image => valid_upload_path(&value),
    };
    if ok {
        return Ok(value);
    }
    Err(match field.kind {
        Kind::Email => "is not a valid email address",
        Kind::Phone => "may only contain digits, spaces and + - ( ) /",
        Kind::Url => "must be a full link starting with https:// or http://",
        Kind::Image => "must be uploaded through the dashboard",
        _ => "is not valid",
    })
}

pub async fn load(pool: &MySqlPool) -> Result<HashMap<String, String>, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT setting_key, setting_value FROM site_settings")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

pub async fn save(pool: &MySqlPool, values: &[(&str, String)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (key, value) in values {
        sqlx::query(
            "INSERT INTO site_settings (setting_key, setting_value) VALUES (?, ?)
             ON DUPLICATE KEY UPDATE setting_value = VALUES(setting_value)",
        )
        .bind(*key)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Everything the public site may know. Every key is always present; images become absolute URLs.
pub async fn public(pool: &MySqlPool, base: &str) -> Result<BTreeMap<String, String>, sqlx::Error> {
    let stored = load(pool).await?;
    Ok(FIELDS
        .iter()
        .map(|f| {
            let mut v = stored.get(f.key).cloned().unwrap_or_default();
            if f.kind == Kind::Image && v.starts_with('/') {
                v = format!("{base}{v}");
            }
            (f.key.to_string(), v)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(key: &str) -> &'static Field {
        FIELDS.iter().find(|f| f.key == key).unwrap()
    }

    #[test]
    fn every_field_belongs_to_a_known_group() {
        for f in FIELDS.iter() {
            assert!(GROUPS.iter().any(|g| g.0 == f.group), "{} has unknown group {}", f.key, f.group);
        }
        let mut keys: Vec<_> = FIELDS.iter().map(|f| f.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), FIELDS.len(), "duplicate keys");
    }

    #[test]
    fn validation() {
        assert_eq!(clean(field("site_name"), "  আজাদির   কথা ").unwrap(), "আজাদির কথা");
        assert_eq!(clean(field("contact_email"), "").unwrap(), "");
        assert!(clean(field("contact_email"), "not-an-email").is_err());
        assert!(clean(field("facebook_url"), "javascript:alert(1)").is_err());
        assert!(clean(field("facebook_url"), "https://facebook.com/x").is_ok());
        assert!(clean(field("logo"), "https://evil.com/logo.png").is_err());
        assert!(clean(field("logo"), "/uploads/2026/10/abc.png").is_ok());
        assert!(clean(field("site_name"), &"a".repeat(121)).is_err());
        assert_eq!(clean(field("office_address"), "House 1\r\nRoad 2").unwrap(), "House 1\nRoad 2");
    }
}
