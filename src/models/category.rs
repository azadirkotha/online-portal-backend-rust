use serde::Serialize;
use sqlx::MySqlPool;

use super::sub_category::{self, PublicSubCategory};

/// Used both for the create/edit form and as the row shape for single-category lookups.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct CategoryInput {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub sort_order: i32,
    pub is_active: bool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct CategoryRow {
    pub id: i32,
    pub name: String,
    pub slug: String,
    pub sort_order: i32,
    pub is_active: bool,
    pub sub_count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PublicCategory {
    pub id: i32,
    pub name: String,
    pub slug: String,
    pub description: String,
    #[sqlx(skip)]
    pub sub_categories: Vec<PublicSubCategory>,
}

// ----- admin -----

pub async fn list(pool: &MySqlPool) -> Result<Vec<CategoryRow>, sqlx::Error> {
    sqlx::query_as::<_, CategoryRow>(
        "SELECT c.id, c.name, c.slug, c.sort_order, c.is_active,
                (SELECT COUNT(*) FROM sub_categories s WHERE s.category_id = c.id) AS sub_count
           FROM categories c
          ORDER BY c.sort_order, c.name",
    )
    .fetch_all(pool)
    .await
}

pub async fn find(pool: &MySqlPool, id: i32) -> Result<Option<CategoryInput>, sqlx::Error> {
    sqlx::query_as::<_, CategoryInput>(
        "SELECT name, slug, description, sort_order, is_active FROM categories WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// (id, name) pairs for dropdowns.
pub async fn options(pool: &MySqlPool) -> Result<Vec<(i32, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, name FROM categories ORDER BY name").fetch_all(pool).await
}

pub async fn create(pool: &MySqlPool, input: &CategoryInput) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "INSERT INTO categories (name, slug, description, sort_order, is_active) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&input.name)
    .bind(&input.slug)
    .bind(&input.description)
    .bind(input.sort_order)
    .bind(input.is_active)
    .execute(pool)
    .await
    .map(|r| r.last_insert_id())
}

pub async fn update(pool: &MySqlPool, id: i32, input: &CategoryInput) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE categories SET name = ?, slug = ?, description = ?, sort_order = ?, is_active = ? WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.slug)
    .bind(&input.description)
    .bind(input.sort_order)
    .bind(input.is_active)
    .bind(id)
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn delete(pool: &MySqlPool, id: i32) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM categories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

// ----- public API (active records only) -----

pub async fn public_tree(pool: &MySqlPool) -> Result<Vec<PublicCategory>, sqlx::Error> {
    let mut categories = sqlx::query_as::<_, PublicCategory>(
        "SELECT id, name, slug, description FROM categories WHERE is_active = 1 ORDER BY sort_order, name",
    )
    .fetch_all(pool)
    .await?;

    let subs = sub_category::public_all(pool).await?;
    for cat in &mut categories {
        cat.sub_categories = subs.iter().filter(|s| s.category_id == cat.id).cloned().collect();
    }
    Ok(categories)
}

pub async fn public_by_slug(pool: &MySqlPool, slug: &str) -> Result<Option<PublicCategory>, sqlx::Error> {
    let cat = sqlx::query_as::<_, PublicCategory>(
        "SELECT id, name, slug, description FROM categories WHERE slug = ? AND is_active = 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;

    match cat {
        Some(mut c) => {
            c.sub_categories = sub_category::public_for(pool, c.id).await?;
            Ok(Some(c))
        }
        None => Ok(None),
    }
}
