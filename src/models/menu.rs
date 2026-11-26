use serde::{Deserialize, Serialize};
use sqlx::{MySql, MySqlPool, Transaction};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Every navigation list the site can have. To add another one (e.g. a mobile menu), add a line here.
pub const LOCATIONS: [(&str, &str); 4] = [
    ("header", "Main menu (header)"),
    ("footer_quick", "Footer: quick links"),
    ("footer_sections", "Footer: sections"),
    ("footer_about", "Footer: about us"),
];

pub fn location_label(key: &str) -> Option<&'static str> {
    LOCATIONS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l)
}

/// Custom links end up in `href` attributes on the public site, so only safe forms are accepted:
/// a site path (`/about`), `http(s)://...`, `mailto:` or `tel:`. Never `javascript:` / `data:`.
pub fn valid_link(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || u.len() > 500 || u.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let site_path = u.starts_with('/') && !u.starts_with("//") && !u.starts_with("/\\");
    site_path || u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:") || u.starts_with("tel:")
}

// ======================= admin side =======================

#[derive(Debug, sqlx::FromRow)]
pub struct ItemRow {
    pub id: i32,
    pub parent_id: Option<i32>,
    pub kind: String,
    pub label: String,
    pub url: String,
    pub open_in_new_tab: bool,
    pub is_active: bool,
    pub category_name: Option<String>,
    pub sub_name: Option<String>,
    pub sub_parent_name: Option<String>,
}

/// One entry of the tree shown in the builder.
#[derive(Debug)]
pub struct Node {
    pub id: i32,
    pub kind: String,
    pub display: String,
    pub detail: String,
    pub active: bool,
    pub new_tab: bool,
    pub children: Vec<Node>,
}

impl Node {
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_str() {
            "category" => "Category",
            "sub_category" => "Sub category",
            _ => "Link",
        }
    }

    fn from_row(r: &ItemRow) -> Node {
        let source = r.category_name.clone().or_else(|| r.sub_name.clone()).unwrap_or_default();
        let display = if r.label.trim().is_empty() { source } else { r.label.clone() };
        let detail = match r.kind.as_str() {
            "link" => r.url.clone(),
            "sub_category" => format!("in {}", r.sub_parent_name.clone().unwrap_or_default()),
            _ => String::new(),
        };
        Node { id: r.id, kind: r.kind.clone(), display, detail, active: r.is_active, new_tab: r.open_in_new_tab, children: Vec::new() }
    }
}

pub async fn items(pool: &MySqlPool, location: &str) -> Result<Vec<ItemRow>, sqlx::Error> {
    sqlx::query_as::<_, ItemRow>(
        "SELECT m.id, m.parent_id, m.kind, m.label, m.url, m.open_in_new_tab, m.is_active,
                c.name AS category_name, sc.name AS sub_name, scc.name AS sub_parent_name
           FROM menu_items m
           LEFT JOIN categories c ON c.id = m.category_id
           LEFT JOIN sub_categories sc ON sc.id = m.sub_category_id
           LEFT JOIN categories scc ON scc.id = sc.category_id
          WHERE m.location = ?
          ORDER BY m.position, m.id",
    )
    .bind(location)
    .fetch_all(pool)
    .await
}

/// Rows are ordered by (position, id); children are attached to their top-level parent.
pub fn build_tree(rows: Vec<ItemRow>) -> Vec<Node> {
    let ids: HashSet<i32> = rows.iter().map(|r| r.id).collect();
    let mut children: HashMap<i32, Vec<Node>> = HashMap::new();
    let mut top: Vec<Node> = Vec::new();

    for r in &rows {
        let node = Node::from_row(r);
        match r.parent_id.filter(|p| ids.contains(p)) {
            Some(parent) => children.entry(parent).or_default().push(node),
            None => top.push(node),
        }
    }
    for n in &mut top {
        n.children = children.remove(&n.id).unwrap_or_default();
    }
    top
}

/// Categories that are not in this menu yet.
pub async fn available_categories(pool: &MySqlPool, location: &str) -> Result<Vec<(i32, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name FROM categories
          WHERE id NOT IN (SELECT category_id FROM menu_items WHERE location = ? AND category_id IS NOT NULL)
          ORDER BY sort_order, name",
    )
    .bind(location)
    .fetch_all(pool)
    .await
}

/// (id, category id, name, category name) for sub categories that are not in this menu yet.
pub async fn available_sub_categories(
    pool: &MySqlPool,
    location: &str,
) -> Result<Vec<(i32, i32, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT s.id, s.category_id, s.name, c.name
           FROM sub_categories s JOIN categories c ON c.id = s.category_id
          WHERE s.id NOT IN (SELECT sub_category_id FROM menu_items WHERE location = ? AND sub_category_id IS NOT NULL)
          ORDER BY c.sort_order, c.name, s.sort_order, s.name",
    )
    .bind(location)
    .fetch_all(pool)
    .await
}

async fn next_position(tx: &mut Transaction<'_, MySql>, location: &str, parent: Option<i32>) -> Result<i32, sqlx::Error> {
    let n: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(MAX(position), -1) + 1 AS SIGNED) FROM menu_items WHERE location = ? AND parent_id <=> ?",
    )
    .bind(location)
    .bind(parent)
    .fetch_one(&mut **tx)
    .await?;
    Ok(n as i32)
}

pub async fn add_categories(pool: &MySqlPool, location: &str, ids: &[i32]) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut added = 0;
    for id in ids {
        let pos = next_position(&mut tx, location, None).await?;
        added += sqlx::query(
            "INSERT IGNORE INTO menu_items (location, parent_id, kind, category_id, position)
             SELECT ?, NULL, 'category', c.id, ? FROM categories c WHERE c.id = ?",
        )
        .bind(location)
        .bind(pos)
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(added)
}

/// A sub category goes into its own category's dropdown when that category is already in this menu;
/// otherwise it becomes a top-level item.
pub async fn add_sub_categories(pool: &MySqlPool, location: &str, ids: &[i32]) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut added = 0;
    for id in ids {
        let parent: Option<i32> = sqlx::query_scalar(
            "SELECT m.id FROM sub_categories s
               JOIN menu_items m ON m.location = ? AND m.category_id = s.category_id AND m.parent_id IS NULL
              WHERE s.id = ?",
        )
        .bind(location)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let pos = next_position(&mut tx, location, parent).await?;
        added += sqlx::query(
            "INSERT IGNORE INTO menu_items (location, parent_id, kind, sub_category_id, position)
             SELECT ?, ?, 'sub_category', s.id, ? FROM sub_categories s WHERE s.id = ?",
        )
        .bind(location)
        .bind(parent)
        .bind(pos)
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(added)
}

pub async fn add_link(pool: &MySqlPool, location: &str, label: &str, url: &str, new_tab: bool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let pos = next_position(&mut tx, location, None).await?;
    sqlx::query(
        "INSERT INTO menu_items (location, parent_id, kind, label, url, open_in_new_tab, position)
         VALUES (?, NULL, 'link', ?, ?, ?, ?)",
    )
    .bind(location)
    .bind(label)
    .bind(url)
    .bind(new_tab)
    .bind(pos)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

#[derive(Debug, sqlx::FromRow)]
pub struct ItemFull {
    pub id: i32,
    pub location: String,
    pub kind: String,
    pub label: String,
    pub url: String,
    pub open_in_new_tab: bool,
    pub is_active: bool,
    pub source_name: Option<String>,
}

pub async fn find(pool: &MySqlPool, id: i32) -> Result<Option<ItemFull>, sqlx::Error> {
    sqlx::query_as::<_, ItemFull>(
        "SELECT m.id, m.location, m.kind, m.label, m.url, m.open_in_new_tab, m.is_active,
                COALESCE(c.name, sc.name) AS source_name
           FROM menu_items m
           LEFT JOIN categories c ON c.id = m.category_id
           LEFT JOIN sub_categories sc ON sc.id = m.sub_category_id
          WHERE m.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn update(pool: &MySqlPool, id: i32, label: &str, url: &str, new_tab: bool, active: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE menu_items
            SET label = ?, url = CASE WHEN kind = 'link' THEN ? ELSE url END, open_in_new_tab = ?, is_active = ?
          WHERE id = ?",
    )
    .bind(label)
    .bind(url)
    .bind(new_tab)
    .bind(active)
    .bind(id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Dropdown items go with their parent (ON DELETE CASCADE).
pub async fn delete(pool: &MySqlPool, id: i32) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM menu_items WHERE id = ?").bind(id).execute(pool).await.map(|r| r.rows_affected())
}

/// What the drag-and-drop builder posts: top-level items, each with its dropdown items.
#[derive(Debug, Deserialize)]
pub struct TreeNode {
    pub id: i32,
    #[serde(default)]
    pub children: Vec<i32>,
}

/// Applies the new order. Ids from other menus or unknown ids are ignored, every item is placed
/// at most once, and nothing can end up deeper than two levels.
pub async fn save_order(pool: &MySqlPool, location: &str, tree: &[TreeNode]) -> Result<(), sqlx::Error> {
    let known: HashSet<i32> = sqlx::query_scalar("SELECT id FROM menu_items WHERE location = ?")
        .bind(location)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();

    let mut tx = pool.begin().await?;
    let mut seen: HashSet<i32> = HashSet::new();
    let mut top_pos = 0;

    for node in tree {
        if !known.contains(&node.id) || !seen.insert(node.id) {
            continue;
        }
        sqlx::query("UPDATE menu_items SET parent_id = NULL, position = ? WHERE id = ? AND location = ?")
            .bind(top_pos)
            .bind(node.id)
            .bind(location)
            .execute(&mut *tx)
            .await?;
        top_pos += 1;

        let mut child_pos = 0;
        for child in &node.children {
            if !known.contains(child) || !seen.insert(*child) {
                continue;
            }
            sqlx::query("UPDATE menu_items SET parent_id = ?, position = ? WHERE id = ? AND location = ?")
                .bind(node.id)
                .bind(child_pos)
                .bind(child)
                .bind(location)
                .execute(&mut *tx)
                .await?;
            child_pos += 1;
        }
    }
    tx.commit().await
}

// ======================= public API =======================

/// Where categories / sub categories live on the Vue site. `{slug}` and `{category}` are replaced.
pub struct UrlPatterns<'a> {
    pub category: &'a str,
    pub sub_category: &'a str,
}

#[derive(Debug, Serialize)]
pub struct PublicItem {
    pub id: i32,
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    pub url: String,
    pub new_tab: bool,
    /// category / sub category slug (lets the frontend build its own route)
    pub slug: Option<String>,
    /// for sub categories: the parent category's slug
    pub category_slug: Option<String>,
    pub children: Vec<PublicItem>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct PublicRow {
    pub id: i32,
    pub parent_id: Option<i32>,
    pub location: String,
    pub kind: String,
    pub label: String,
    pub url: String,
    pub open_in_new_tab: bool,
    pub category_name: Option<String>,
    pub category_slug: Option<String>,
    pub category_active: Option<bool>,
    pub sub_name: Option<String>,
    pub sub_slug: Option<String>,
    pub sub_active: Option<bool>,
    pub sub_parent_slug: Option<String>,
    pub sub_parent_active: Option<bool>,
}

fn fill(pattern: &str, slug: &str, category: &str) -> String {
    pattern.replace("{slug}", slug).replace("{category}", category)
}

fn to_item(r: &PublicRow, p: &UrlPatterns) -> Option<PublicItem> {
    let custom = r.label.trim();
    let pick = |name: &str| if custom.is_empty() { name.to_string() } else { custom.to_string() };

    match r.kind.as_str() {
        "category" => {
            if r.category_active != Some(true) {
                return None;
            }
            let slug = r.category_slug.clone()?;
            Some(PublicItem {
                id: r.id,
                kind: "category".into(),
                label: pick(r.category_name.as_deref()?),
                url: fill(p.category, &slug, ""),
                new_tab: r.open_in_new_tab,
                slug: Some(slug),
                category_slug: None,
                children: Vec::new(),
            })
        }
        "sub_category" => {
            if r.sub_active != Some(true) || r.sub_parent_active != Some(true) {
                return None;
            }
            let slug = r.sub_slug.clone()?;
            let parent = r.sub_parent_slug.clone()?;
            Some(PublicItem {
                id: r.id,
                kind: "sub_category".into(),
                label: pick(r.sub_name.as_deref()?),
                url: fill(p.sub_category, &slug, &parent),
                new_tab: r.open_in_new_tab,
                slug: Some(slug),
                category_slug: Some(parent),
                children: Vec::new(),
            })
        }
        "link" => {
            if custom.is_empty() || !valid_link(&r.url) {
                return None;
            }
            Some(PublicItem {
                id: r.id,
                kind: "link".into(),
                label: custom.to_string(),
                url: r.url.trim().to_string(),
                new_tab: r.open_in_new_tab,
                slug: None,
                category_slug: None,
                children: Vec::new(),
            })
        }
        _ => None,
    }
}

/// Rows must be ordered by (position, id). Hidden items, items whose category is hidden, and the
/// children of a hidden item are left out.
pub fn assemble(rows: Vec<PublicRow>, patterns: &UrlPatterns) -> BTreeMap<String, Vec<PublicItem>> {
    let mut out: BTreeMap<String, Vec<PublicItem>> =
        LOCATIONS.iter().map(|(k, _)| (k.to_string(), Vec::new())).collect();
    let mut children: HashMap<i32, Vec<PublicItem>> = HashMap::new();
    let mut top: Vec<(String, i32, PublicItem)> = Vec::new();

    for r in &rows {
        let Some(item) = to_item(r, patterns) else { continue };
        match r.parent_id {
            Some(parent) => children.entry(parent).or_default().push(item),
            None => top.push((r.location.clone(), r.id, item)),
        }
    }
    for (location, id, mut item) in top {
        item.children = children.remove(&id).unwrap_or_default();
        out.entry(location).or_default().push(item);
    }
    out
}

pub async fn public_menus(pool: &MySqlPool, patterns: &UrlPatterns<'_>) -> Result<BTreeMap<String, Vec<PublicItem>>, sqlx::Error> {
    let rows = sqlx::query_as::<_, PublicRow>(
        "SELECT m.id, m.parent_id, m.location, m.kind, m.label, m.url, m.open_in_new_tab,
                c.name AS category_name, c.slug AS category_slug, c.is_active AS category_active,
                sc.name AS sub_name, sc.slug AS sub_slug, sc.is_active AS sub_active,
                scc.slug AS sub_parent_slug, scc.is_active AS sub_parent_active
           FROM menu_items m
           LEFT JOIN categories c ON c.id = m.category_id
           LEFT JOIN sub_categories sc ON sc.id = m.sub_category_id
           LEFT JOIN categories scc ON scc.id = sc.category_id
          WHERE m.is_active = 1
          ORDER BY m.position, m.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(assemble(rows, patterns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_validation() {
        for ok in ["/about", "/news/২০২৬", "https://example.com/a?b=1", "http://x.org", "mailto:a@b.com", "tel:+8801700000000"] {
            assert!(valid_link(ok), "{ok} should be accepted");
        }
        for bad in ["", "javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,x", "//evil.com", "/\\evil.com", "about", "https://a b.com", " /x y", "vbscript:x", "ftp://x"] {
            assert!(!valid_link(bad), "{bad:?} should be rejected");
        }
    }

    fn row(id: i32, parent: Option<i32>, kind: &str) -> PublicRow {
        PublicRow {
            id, parent_id: parent, location: "header".into(), kind: kind.into(), label: String::new(), url: String::new(),
            open_in_new_tab: false, category_name: None, category_slug: None, category_active: None,
            sub_name: None, sub_slug: None, sub_active: None, sub_parent_slug: None, sub_parent_active: None,
        }
    }

    fn cat(id: i32, parent: Option<i32>, name: &str, slug: &str, active: bool) -> PublicRow {
        PublicRow { category_name: Some(name.into()), category_slug: Some(slug.into()), category_active: Some(active), ..row(id, parent, "category") }
    }

    fn sub(id: i32, parent: Option<i32>, name: &str, slug: &str, cat_slug: &str, active: bool) -> PublicRow {
        PublicRow {
            sub_name: Some(name.into()), sub_slug: Some(slug.into()), sub_active: Some(active),
            sub_parent_slug: Some(cat_slug.into()), sub_parent_active: Some(true), ..row(id, parent, "sub_category")
        }
    }

    const P: UrlPatterns<'static> = UrlPatterns { category: "/{slug}", sub_category: "/{category}/{slug}" };

    #[test]
    fn builds_dropdowns_and_urls() {
        let rows = vec![
            cat(1, None, "রাজনীতি", "politics", true),
            sub(2, Some(1), "নির্বাচন", "election", "politics", true),
            PublicRow { label: "আমাদের সম্পর্কে".into(), url: "/about".into(), ..row(3, None, "link") },
        ];
        let m = assemble(rows, &P);
        let header = &m["header"];
        assert_eq!(header.len(), 2);
        assert_eq!(header[0].url, "/politics");
        assert_eq!(header[0].children[0].url, "/politics/election");
        assert_eq!(header[0].children[0].category_slug.as_deref(), Some("politics"));
        assert_eq!(header[1].label, "আমাদের সম্পর্কে");
        assert!(m["footer_quick"].is_empty(), "every location is always present");
    }

    #[test]
    fn hidden_things_are_left_out() {
        let rows = vec![
            cat(1, None, "Hidden", "hidden", false),
            sub(2, Some(1), "child of hidden", "c", "hidden", true), // parent hidden => gone
            cat(3, None, "Shown", "shown", true),
            sub(4, Some(3), "inactive sub", "s", "shown", false),
            PublicRow { label: "bad".into(), url: "javascript:alert(1)".into(), ..row(5, None, "link") },
        ];
        let header = &assemble(rows, &P)["header"];
        assert_eq!(header.len(), 1);
        assert_eq!(header[0].label, "Shown");
        assert!(header[0].children.is_empty());
    }

    #[test]
    fn label_override_wins() {
        let mut r = cat(1, None, "Politics", "politics", true);
        r.label = "রাজনীতি".into();
        assert_eq!(assemble(vec![r], &P)["header"][0].label, "রাজনীতি");
    }
}
