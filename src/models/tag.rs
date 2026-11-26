use serde::Serialize;
use sqlx::MySqlPool;

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct TagInput {
    pub name: String,
    pub slug: String,
}

/// Same shape for the admin list and the public API.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Tag {
    pub id: i32,
    pub name: String,
    pub slug: String,
}

pub async fn list(pool: &MySqlPool) -> Result<Vec<Tag>, sqlx::Error> {
    sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags ORDER BY name")
        .fetch_all(pool)
        .await
}

pub async fn find(pool: &MySqlPool, id: i32) -> Result<Option<TagInput>, sqlx::Error> {
    sqlx::query_as::<_, TagInput>("SELECT name, slug FROM tags WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn find_by_slug(pool: &MySqlPool, slug: &str) -> Result<Option<Tag>, sqlx::Error> {
    sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags WHERE slug = ?")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn create(pool: &MySqlPool, input: &TagInput) -> Result<u64, sqlx::Error> {
    sqlx::query("INSERT INTO tags (name, slug) VALUES (?, ?)")
        .bind(&input.name)
        .bind(&input.slug)
        .execute(pool)
        .await
        .map(|r| r.last_insert_id())
}

pub async fn update(pool: &MySqlPool, id: i32, input: &TagInput) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tags SET name = ?, slug = ? WHERE id = ?")
        .bind(&input.name)
        .bind(&input.slug)
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn delete(pool: &MySqlPool, id: i32) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}
