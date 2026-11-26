//! Validation + queries for user profiles (self-service account page and the admin user forms).

use chrono::NaiveDate;
use sqlx::MySqlPool;

/// Users with this role get the press-card section in the dashboard.
pub const JOURNALIST_ROLE: &str = "Journalist";

pub const BLOOD_GROUPS: [&str; 8] = ["A+", "A-", "B+", "B-", "AB+", "AB-", "O+", "O-"];

// ---------- small validators / cleaners ----------

/// Only paths produced by our own uploader are accepted for photos, logos and favicons.
pub fn valid_upload_path(p: &str) -> bool {
    p.starts_with("/uploads/")
        && p.len() <= 500
        && !p.contains("..")
        && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
}

pub fn valid_email(e: &str) -> bool {
    let e = e.trim();
    if e.is_empty() || e.len() > 255 || e.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let mut parts = e.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

/// Public contact numbers: digits (ASCII or Bengali), + - ( ) / , and spaces.
pub fn valid_phone(p: &str) -> bool {
    let p = p.trim();
    !p.is_empty()
        && p.chars().count() <= 40
        && p.chars().all(|c| c.is_ascii_digit() || ('\u{09E6}'..='\u{09EF}').contains(&c) || matches!(c, '+' | '-' | ' ' | '(' | ')' | '/' | ','))
}

pub fn valid_http_url(u: &str) -> bool {
    let u = u.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    match rest {
        Some(r) => !r.is_empty() && u.len() <= 500 && !u.chars().any(|c| c.is_whitespace() || c.is_control()),
        None => false,
    }
}

/// Single line: control characters become spaces, runs of whitespace collapse.
pub fn clean_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Several lines: keeps line breaks, drops other control characters.
pub fn clean_multiline(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .to_string()
}

// ---------- staff / press-card details (managed by admins) ----------

/// As typed in the form (also used to show the form again after an error).
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct StaffFields {
    pub photo: String,
    pub bio: String,
    pub designation: String,
    pub blood_group: String,
    pub journalist_number: String,
    pub card_issue_date: String,
    pub card_expiry_date: String,
}

/// Checked and ready for the database.
#[derive(Debug, Clone)]
pub struct StaffParsed {
    pub photo: String,
    pub bio: String,
    pub designation: String,
    pub blood_group: String,
    pub journalist_number: Option<String>,
    pub card_issue_date: Option<String>,
    pub card_expiry_date: Option<String>,
}

fn parse_date(s: &str, what: &'static str) -> Result<Option<NaiveDate>, &'static str> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map(Some).map_err(|_| what)
}

impl StaffFields {
    pub fn parse(&self) -> Result<StaffParsed, &'static str> {
        let photo = self.photo.trim().to_string();
        if !photo.is_empty() && !valid_upload_path(&photo) {
            return Err("The photo must be uploaded through the dashboard.");
        }
        let bio = clean_multiline(&self.bio);
        if bio.chars().count() > 500 {
            return Err("The short bio is too long (max 500 characters).");
        }
        let designation = clean_line(&self.designation);
        if designation.chars().count() > 120 {
            return Err("The designation is too long (max 120 characters).");
        }
        let blood_group = self.blood_group.trim().to_string();
        if !blood_group.is_empty() && !BLOOD_GROUPS.contains(&blood_group.as_str()) {
            return Err("Choose a valid blood group.");
        }

        let number = clean_line(&self.journalist_number);
        let journalist_number = if number.is_empty() {
            None
        } else if number.chars().count() > 50 || !number.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | '_' | ' ')) {
            return Err("The journalist number may contain letters, digits, - and / only (max 50 characters).");
        } else {
            Some(number)
        };

        let issue = parse_date(&self.card_issue_date, "The card issue date is not a valid date.")?;
        let expiry = parse_date(&self.card_expiry_date, "The card expiry date is not a valid date.")?;
        if let (Some(i), Some(e)) = (issue, expiry) {
            if e < i {
                return Err("The card expiry date cannot be before the issue date.");
            }
        }

        Ok(StaffParsed {
            photo,
            bio,
            designation,
            blood_group,
            journalist_number,
            card_issue_date: issue.map(|d| d.format("%Y-%m-%d").to_string()),
            card_expiry_date: expiry.map(|d| d.format("%Y-%m-%d").to_string()),
        })
    }
}

pub async fn staff_fields(pool: &MySqlPool, id: i32) -> Result<Option<StaffFields>, sqlx::Error> {
    sqlx::query_as::<_, StaffFields>(
        "SELECT photo, bio, designation, blood_group,
                COALESCE(journalist_number, '') AS journalist_number,
                COALESCE(DATE_FORMAT(card_issue_date, '%Y-%m-%d'), '') AS card_issue_date,
                COALESCE(DATE_FORMAT(card_expiry_date, '%Y-%m-%d'), '') AS card_expiry_date
           FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Is the card expired today? (false when there is no valid expiry date)
pub fn card_expired(expiry: &str) -> bool {
    NaiveDate::parse_from_str(expiry, "%Y-%m-%d")
        .map(|d| d < chrono::Utc::now().date_naive())
        .unwrap_or(false)
}

// ---------- self-service account ----------

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct ProfileFields {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub photo: String,
    pub bio: String,
}

pub async fn profile(pool: &MySqlPool, id: i32) -> Result<Option<ProfileFields>, sqlx::Error> {
    sqlx::query_as::<_, ProfileFields>("SELECT name, email, phone, photo, bio FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_profile(pool: &MySqlPool, id: i32, p: &ProfileFields) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET name = ?, email = ?, phone = ?, photo = ?, bio = ? WHERE id = ?")
        .bind(&p.name)
        .bind(&p.email)
        .bind(&p.phone)
        .bind(&p.photo)
        .bind(&p.bio)
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn password_hash(pool: &MySqlPool, id: i32) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT password FROM users WHERE id = ?").bind(id).fetch_optional(pool).await
}

pub async fn set_password(pool: &MySqlPool, id: i32, hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password = ? WHERE id = ?").bind(hash).bind(id).execute(pool).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_paths() {
        assert!(valid_upload_path("/uploads/2026/10/abc123.png"));
        for bad in ["", "uploads/a.png", "/uploads/../etc/passwd", "http://evil/uploads/a.png", "/uploads/a b.png", "/other/a.png", "/uploads/a.png?x=1"] {
            assert!(!valid_upload_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn emails() {
        for ok in ["a@b.co", "reporter.name+tag@news-portal.com.bd"] {
            assert!(valid_email(ok), "{ok}");
        }
        for bad in ["", "a", "a@b", "@b.com", "a@@b.com", "a b@c.com", "a@.com", "a@b..com", "a@b.com."] {
            assert!(!valid_email(bad), "{bad:?}");
        }
    }

    #[test]
    fn phones_and_urls() {
        assert!(valid_phone("+880 1711-123456"));
        assert!(valid_phone("০১৭১১-১২৩৪৫৬"));
        assert!(!valid_phone("call me"));
        assert!(!valid_phone(""));
        assert!(valid_http_url("https://facebook.com/azadirkotha"));
        for bad in ["javascript:alert(1)", "ftp://x.com", "https://", "https://a b.com", "facebook.com"] {
            assert!(!valid_http_url(bad), "{bad:?}");
        }
    }

    fn staff() -> StaffFields {
        StaffFields { journalist_number: "BJ-2026/045".into(), card_issue_date: "2026-01-05".into(), card_expiry_date: "2027-01-04".into(), blood_group: "O+".into(), ..Default::default() }
    }

    #[test]
    fn staff_ok_and_normalised() {
        let p = staff().parse().unwrap();
        assert_eq!(p.journalist_number.as_deref(), Some("BJ-2026/045"));
        assert_eq!(p.card_issue_date.as_deref(), Some("2026-01-05"));
        let empty = StaffFields::default().parse().unwrap();
        assert!(empty.journalist_number.is_none() && empty.card_issue_date.is_none());
    }

    #[test]
    fn staff_rejections() {
        let mut s = staff();
        s.card_expiry_date = "2025-12-31".into();
        assert!(s.parse().is_err(), "expiry before issue");
        let mut s = staff();
        s.card_issue_date = "05/01/2026".into();
        assert!(s.parse().is_err(), "wrong date format");
        let mut s = staff();
        s.blood_group = "Z+".into();
        assert!(s.parse().is_err());
        let mut s = staff();
        s.journalist_number = "<script>".into();
        assert!(s.parse().is_err());
        let mut s = staff();
        s.photo = "https://evil.com/a.png".into();
        assert!(s.parse().is_err());
    }

    #[test]
    fn expiry_check() {
        assert!(card_expired("2001-01-01"));
        assert!(!card_expired("2999-01-01"));
        assert!(!card_expired(""));
    }
}
