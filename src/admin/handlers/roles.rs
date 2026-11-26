use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::extract::Form;
use serde::Deserialize;
use sqlx::MySqlPool;
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::helpers::{
    build_layout, flash, forbidden, internal_error, is_unique_violation, not_found, render,
};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::{CheckOption, RoleEditTemplate, RoleNewTemplate, RoleRow, RolesListTemplate};
use crate::models::permissions::SUPER_ADMIN_ROLE;
use crate::state::AppState;

async fn permission_options(pool: &MySqlPool, selected: &[i32]) -> Result<Vec<CheckOption>, sqlx::Error> {
    let rows: Vec<(i32, String)> = sqlx::query_as("SELECT id, name FROM permissions ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, label)| CheckOption { id, label, checked: selected.contains(&id) })
        .collect())
}

pub async fn list(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("role:read") {
        return forbidden();
    }

    let rows: Result<Vec<(i32, String, String, i64, i64)>, _> = sqlx::query_as(
        "SELECT r.id, r.name, r.description,
                (SELECT COUNT(*) FROM role_permissions rp WHERE rp.role_id = r.id),
                (SELECT COUNT(*) FROM user_roles ur WHERE ur.role_id = r.id)
           FROM roles r ORDER BY r.name",
    )
    .fetch_all(&state.db_pool)
    .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };

    let roles = rows
        .into_iter()
        .map(|(id, name, description, perm_count, user_count)| RoleRow {
            id,
            locked: name == SUPER_ADMIN_ROLE,
            name,
            description,
            perm_count,
            user_count,
        })
        .collect();

    let layout = build_layout(&session, &admin).await;
    render(RolesListTemplate { layout, roles, can_write: admin.has("role:write") })
}

pub async fn new_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("role:write") {
        return forbidden();
    }
    let perms = match permission_options(&state.db_pool, &[]).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };
    let layout = build_layout(&session, &admin).await;
    render(RoleNewTemplate { layout, perms })
}

#[derive(Deserialize)]
pub struct RoleForm {
    pub csrf_token: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub permission_ids: Vec<i32>,
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<RoleForm>,
) -> Response {
    if !admin.has("role:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }

    let name = f.name.trim();
    if name.is_empty() || name.len() > 100 || f.description.len() > 255 {
        flash(&session, "Role name is required (max 100 chars); description max 255 chars.").await;
        return Redirect::to("/roles/new").into_response();
    }

    let mut tx = match state.db_pool.begin().await {
        Ok(t) => t,
        Err(e) => return internal_error(e),
    };

    let res = sqlx::query("INSERT INTO roles (name, description) VALUES (?, ?)")
        .bind(name)
        .bind(f.description.trim())
        .execute(&mut *tx)
        .await;
    let role_id = match res {
        Ok(r) => r.last_insert_id() as i32,
        Err(e) if is_unique_violation(&e) => {
            flash(&session, "A role with that name already exists.").await;
            return Redirect::to("/roles/new").into_response();
        }
        Err(e) => return internal_error(e),
    };

    for pid in &f.permission_ids {
        if let Err(e) = sqlx::query("INSERT IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)")
            .bind(role_id)
            .bind(pid)
            .execute(&mut *tx)
            .await
        {
            return internal_error(e);
        }
    }

    if let Err(e) = tx.commit().await {
        return internal_error(e);
    }

    flash(&session, &format!("Role '{name}' created.")).await;
    Redirect::to("/roles").into_response()
}

pub async fn edit_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    if !admin.has("role:write") {
        return forbidden();
    }

    let role: Result<Option<(String, String)>, _> =
        sqlx::query_as("SELECT name, description FROM roles WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db_pool)
            .await;
    let (name, description) = match role {
        Ok(Some(r)) => r,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };

    if name == SUPER_ADMIN_ROLE {
        flash(&session, "The Super Admin role is protected and cannot be edited.").await;
        return Redirect::to("/roles").into_response();
    }

    let selected: Vec<i32> = match sqlx::query_scalar("SELECT permission_id FROM role_permissions WHERE role_id = ?")
        .bind(id)
        .fetch_all(&state.db_pool)
        .await
    {
        Ok(v) => v,
        Err(e) => return internal_error(e),
    };
    let perms = match permission_options(&state.db_pool, &selected).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };

    let layout = build_layout(&session, &admin).await;
    render(RoleEditTemplate {
        layout,
        id,
        name,
        description,
        perms,
    })
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<RoleForm>,
) -> Response {
    if !admin.has("role:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }

    let back = format!("/roles/{id}/edit");

    let current: Result<Option<(String,)>, _> = sqlx::query_as("SELECT name FROM roles WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await;
    let current_name = match current {
        Ok(Some((n,))) => n,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };

    if current_name == SUPER_ADMIN_ROLE {
        flash(&session, "The Super Admin role is protected and cannot be edited.").await;
        return Redirect::to("/roles").into_response();
    }

    let name = f.name.trim();
    if name.is_empty() || name.len() > 100 || f.description.len() > 255 {
        flash(&session, "Role name is required (max 100 chars); description max 255 chars.").await;
        return Redirect::to(&back).into_response();
    }
    if name == SUPER_ADMIN_ROLE {
        flash(&session, "That role name is reserved.").await;
        return Redirect::to(&back).into_response();
    }

    let mut tx = match state.db_pool.begin().await {
        Ok(t) => t,
        Err(e) => return internal_error(e),
    };

    match sqlx::query("UPDATE roles SET name = ?, description = ? WHERE id = ?")
        .bind(name)
        .bind(f.description.trim())
        .bind(id)
        .execute(&mut *tx)
        .await
    {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            flash(&session, "A role with that name already exists.").await;
            return Redirect::to(&back).into_response();
        }
        Err(e) => return internal_error(e),
    }

    if let Err(e) = sqlx::query("DELETE FROM role_permissions WHERE role_id = ?").bind(id).execute(&mut *tx).await {
        return internal_error(e);
    }
    for pid in &f.permission_ids {
        if let Err(e) = sqlx::query("INSERT IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)")
            .bind(id)
            .bind(pid)
            .execute(&mut *tx)
            .await
        {
            return internal_error(e);
        }
    }

    if let Err(e) = tx.commit().await {
        return internal_error(e);
    }

    flash(&session, "Role updated. Changes apply to users immediately.").await;
    Redirect::to("/roles").into_response()
}

#[derive(Deserialize)]
pub struct CsrfOnly {
    pub csrf_token: String,
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !admin.has("role:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }

    let name: Result<Option<(String,)>, _> = sqlx::query_as("SELECT name FROM roles WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await;
    match name {
        Ok(Some((n,))) if n == SUPER_ADMIN_ROLE => {
            flash(&session, "The Super Admin role cannot be deleted.").await;
            return Redirect::to("/roles").into_response();
        }
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    }

    // ON DELETE CASCADE removes the role's permission and user links.
    if let Err(e) = sqlx::query("DELETE FROM roles WHERE id = ?").bind(id).execute(&state.db_pool).await {
        return internal_error(e);
    }

    flash(&session, "Role deleted.").await;
    Redirect::to("/roles").into_response()
}
