use sqlx::MySqlPool;

/// This role always holds every permission and cannot be edited or deleted from the dashboard.
pub const SUPER_ADMIN_ROLE: &str = "Super Admin";

/// Effective permissions = permissions granted directly to the user
/// UNION permissions granted through any of the user's roles.
pub async fn effective_for_user(pool: &MySqlPool, user_id: i32) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT p.name FROM permissions p
           JOIN user_permissions up ON up.permission_id = p.id
          WHERE up.user_id = ?
         UNION
         SELECT p.name FROM permissions p
           JOIN role_permissions rp ON rp.permission_id = p.id
           JOIN user_roles ur ON ur.role_id = rp.role_id
          WHERE ur.user_id = ?",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
}
