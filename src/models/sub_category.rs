use serde::Serialize;
use sqlx::MySqlPool;

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct SubCategoryInput {
    pub category_id: i32,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub sort_order: i32,
    pub is_active: bool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SubCategoryRow {
    pub id: i32,
    pub name: String,
    pub slug: String,
    pub category_name: String,
    pub sort_order: i32,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PublicSubCategory {
    pub id: i32,
    #[serde(skip)]
    pub category_id: i32,
    pub name: String,
    pub slug: String,
    pub description: String,
}

// ----- admin -----

pub async fn list(pool: &MySqlPool) -> Result<Vec<SubCategoryRow>, sqlx::Error> {
    sqlx::query_as::<_, SubCategoryRow>(
        "SELECT s.id, s.name, s.slug, c.name AS category_name, s.sort_order, s.is_active
           FROM sub_categories s
           JOIN categories c ON c.id = s.category_id
          ORDER BY c.name, s.sort_order, s.name",
    )
    .fetch_all(pool)
    .await
}

pub async fn find(pool: &MySqlPool, id: i32) -> Result<Option<SubCategoryInput>, sqlx::Error> {
    sqlx::query_as::<_, SubCategoryInput>(
        "SELECT category_id, name, slug, description, sort_order, is_active FROM sub_categories WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn create(pool: &MySqlPool, input: &SubCategoryInput) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "INSERT INTO sub_categories (category_id, name, slug, description, sort_order, is_active)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(input.category_id)
    .bind(&input.name)
    .bind(&input.slug)
    .bind(&input.description)
    .bind(input.sort_order)
    .bind(input.is_active)
    .execute(pool)
    .await
    .map(|r| r.last_insert_id())
}

pub async fn update(pool: &MySqlPool, id: i32, input: &SubCategoryInput) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sub_categories
            SET category_id = ?, name = ?, slug = ?, description = ?, sort_order = ?, is_active = ?
          WHERE id = ?",
    )
    .bind(input.category_id)
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
    sqlx::query("DELETE FROM sub_categories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

// ----- public API (active records only) -----

pub async fn public_all(pool: &MySqlPool) -> Result<Vec<PublicSubCategory>, sqlx::Error> {
    sqlx::query_as::<_, PublicSubCategory>(
        "SELECT id, category_id, name, slug, description
           FROM sub_categories WHERE is_active = 1 ORDER BY sort_order, name",
    )
    .fetch_all(pool)
    .await
}

pub async fn public_for(pool: &MySqlPool, category_id: i32) -> Result<Vec<PublicSubCategory>, sqlx::Error> {
    sqlx::query_as::<_, PublicSubCategory>(
        "SELECT id, category_id, name, slug, description
           FROM sub_categories WHERE category_id = ? AND is_active = 1 ORDER BY sort_order, name",
    )
    .bind(category_id)
    .fetch_all(pool)
    .await
}
