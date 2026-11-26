//! The site name and logo from the Site settings page, for the sidebar, the top bar and the
//! sign-in page.
//!
//! The values live in a small in-memory cache so templates can read them without a database call:
//!   * `start()` loads them once when the admin starts (called from `create_routes`),
//!   * `refresh()` reloads them right after the settings are saved.

use std::sync::RwLock;

use sqlx::MySqlPool;

use super::views::{Layout, LoginTemplate};
use crate::models::site_settings;

#[derive(Clone, Default)]
struct Brand {
    name: String,
    logo: String,
}

static BRAND: RwLock<Brand> = RwLock::new(Brand { name: String::new(), logo: String::new() });

fn current() -> Brand {
    BRAND.read().map(|b| b.clone()).unwrap_or_default()
}

/// The admin's content-security-policy only allows images from its own origin, so a logo saved as
/// an absolute URL into /uploads/ is reduced to its path.
fn local_url(v: &str) -> String {
    let v = v.trim();
    if v.starts_with("http://") || v.starts_with("https://") {
        if let Some(i) = v.find("/uploads/") {
            return v[i..].to_string();
        }
    }
    v.to_string()
}

/// Reloads the name and logo from the settings table.
pub async fn refresh(pool: &MySqlPool) {
    match site_settings::load(pool).await {
        Ok(values) => {
            let name = values.get("site_name").map(|s| s.trim().to_string()).unwrap_or_default();
            let logo = values.get("logo").map(|s| local_url(s)).unwrap_or_default();
            if let Ok(mut b) = BRAND.write() {
                *b = Brand { name, logo };
            }
        }
        Err(e) => eprintln!("[admin] could not load the site name and logo: {e:?}"),
    }
}

/// Loads the name and logo in the background; call it once while the server is starting.
pub fn start(pool: MySqlPool) {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(async move { refresh(&pool).await });
        }
        Err(_) => eprintln!("[admin] brand::start was called outside the async runtime"),
    }
}

fn logo() -> String {
    current().logo
}

fn site_name() -> String {
    let n = current().name;
    if n.is_empty() { "Newsroom".to_string() } else { n }
}

fn site_initial() -> String {
    site_name().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "N".to_string())
}

// Template helpers: `layout.logo()`, `layout.site_name()`, `layout.site_initial()` in the
// dashboard pages and `self.logo()` etc. on the sign-in page.
impl Layout {
    pub fn logo(&self) -> String { logo() }
    pub fn site_name(&self) -> String { site_name() }
    pub fn site_initial(&self) -> String { site_initial() }
}

impl LoginTemplate {
    pub fn logo(&self) -> String { logo() }
    pub fn site_name(&self) -> String { site_name() }
    pub fn site_initial(&self) -> String { site_initial() }
}