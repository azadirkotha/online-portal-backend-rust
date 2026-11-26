use sqlx::MySqlPool;

use crate::config::AppConfig;
use crate::models::permissions::SUPER_ADMIN_ROLE;
use crate::security;

/// Idempotent: safe to run on every start.
pub async fn run(pool: &MySqlPool, config: &AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    // 1. The Super Admin role exists...
    sqlx::query("INSERT IGNORE INTO roles (name, description) VALUES (?, ?)")
        .bind(SUPER_ADMIN_ROLE)
        .bind("Full access to everything. Cannot be edited or deleted.")
        .execute(pool)
        .await?;

    // 2. ...and always holds every permission (picks up permissions added by future migrations).
    sqlx::query(
        "INSERT IGNORE INTO role_permissions (role_id, permission_id)
         SELECT r.id, p.id FROM roles r CROSS JOIN permissions p WHERE r.name = ?",
    )
    .bind(SUPER_ADMIN_ROLE)
    .execute(pool)
    .await?;

    // 3. Bootstrap admin user from .env
    let Some(seed) = &config.admin_seed else {
        println!("[seed] ADMIN_USERNAME/ADMIN_PASSWORD not set, skipping admin user seed.");
        return Ok(());
    };

    let existing: Option<(i32,)> = sqlx::query_as("SELECT id FROM users WHERE username = ?")
        .bind(&seed.username)
        .fetch_optional(pool)
        .await?;

    let user_id: i32 = match existing {
        Some((id,)) => {
            println!("[seed] User '{}' already exists, ensuring admin access (password untouched).", seed.username);
            id
        }
        None => {
            let hash = security::hash_password(&seed.password).map_err(|e| e.to_string())?;
            let res = sqlx::query(
                "INSERT INTO users (name, email, phone, username, password, status) VALUES (?, ?, ?, ?, ?, 1)",
            )
            .bind(&seed.name)
            .bind(&seed.email)
            .bind(&seed.phone)
            .bind(&seed.username)
            .bind(hash)
            .execute(pool)
            .await?;
            println!("[seed] Created admin user '{}'.", seed.username);
            res.last_insert_id() as i32
        }
    };

    // Make sure the account is active
    sqlx::query("UPDATE users SET status = 1 WHERE id = ?").bind(user_id).execute(pool).await?;

    // Super Admin role
    sqlx::query(
        "INSERT IGNORE INTO user_roles (user_id, role_id) SELECT ?, id FROM roles WHERE name = ?",
    )
    .bind(user_id)
    .bind(SUPER_ADMIN_ROLE)
    .execute(pool)
    .await?;

    // Direct grants in user_permissions: every permission that exists today.
    // (user_id is an integer from the DB, so formatting it into the SQL is injection-safe.)
    sqlx::query(&format!(
        "INSERT IGNORE INTO user_permissions (user_id, permission_id) SELECT {user_id}, id FROM permissions"
    ))
    .execute(pool)
    .await?;

    Ok(())
}
