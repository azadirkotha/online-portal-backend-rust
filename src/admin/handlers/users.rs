use std::collections::HashMap;
use std::path::PathBuf;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::extract::Form;
use serde::Deserialize;
use sqlx::{MySql, MySqlPool, QueryBuilder};
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::helpers::{
    build_layout, flash, forbidden, internal_error, is_unique_violation, not_found, render,
};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::{Layout, UserRow, UsersListTemplate};
use crate::models::profile::{self, StaffFields, BLOOD_GROUPS, JOURNALIST_ROLE};

pub struct BloodOption {
    pub value: &'static str,
    pub selected: bool,
}

fn blood_options(selected: &str) -> Vec<BloodOption> {
    BLOOD_GROUPS.iter().map(|b| BloodOption { value: b, selected: *b == selected }).collect()
}
use crate::security;
use crate::state::AppState;

const MIN_PASSWORD_LEN: usize = 10;
/// Where journalist application documents are stored (must match the API controller).
const PRIVATE_DIR: &str = "private_uploads/applications";

// ---------- view models ----------

pub struct RoleOption {
    pub id: i32,
    pub label: String,
    pub checked: bool,
    /// the Journalist role reveals the press-card section of the form
    pub is_journalist: bool,
}

/// Everything shown in the user form (also used to show it again after an error).
#[derive(Default)]
pub struct UserValues {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub username: String,
    pub active: bool,
    pub staff: StaffFields,
}

#[derive(Template)]
#[template(path = "admin/users/new.html")]
struct NewTemplate {
    layout: Layout,
    v: UserValues,
    roles: Vec<RoleOption>,
    can_assign: bool,
    show_press_card: bool,
    blood_groups: Vec<BloodOption>,
}

#[derive(Template)]
#[template(path = "admin/users/edit.html")]
struct EditTemplate {
    layout: Layout,
    id: i32,
    v: UserValues,
    is_self: bool,
    roles: Vec<RoleOption>,
    can_assign: bool,
    can_delete: bool,
    show_press_card: bool,
    blood_groups: Vec<BloodOption>,
}

async fn role_options(pool: &MySqlPool, selected: &[i32]) -> Result<Vec<RoleOption>, sqlx::Error> {
    let rows: Vec<(i32, String)> = sqlx::query_as("SELECT id, name FROM roles ORDER BY name").fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(id, label)| RoleOption { id, checked: selected.contains(&id), is_journalist: label == JOURNALIST_ROLE, label })
        .collect())
}

fn dup_message(e: &sqlx::Error) -> &'static str {
    let m = e.as_database_error().map(|d| d.message().to_string()).unwrap_or_default();
    if m.contains("journalist_number") {
        "That journalist number is already assigned to someone else."
    } else if m.contains("email") {
        "That email address is already in use."
    } else {
        "That username is already in use."
    }
}

// ---------- list ----------

const PER_PAGE: i64 = 20;

#[derive(Deserialize)]
pub struct ListQuery {
    pub q: Option<String>,
    pub page: Option<i64>,
}

/// `%text%` for LIKE, with the LIKE wildcards in the text escaped.
fn like(s: &str) -> String {
    let mut out = String::from("%");
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Adds the search conditions (name, username, email, phone, journalist number).
/// Bengali digits are matched as well as their ASCII form.
fn push_filter(qb: &mut QueryBuilder<'_, MySql>, q: &str) {
    if q.is_empty() {
        return;
    }
    let a = like(q);
    let b = like(&crate::models::journalist::ascii_digits(q));
    qb.push(" AND (");
    let fields = ["name", "username", "email", "phone", "journalist_number"];
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            qb.push(" OR ");
        }
        qb.push(*f).push(" LIKE ").push_bind(a.clone()).push(" OR ").push(*f).push(" LIKE ").push_bind(b.clone());
    }
    qb.push(")");
}

pub async fn list(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Query(params): Query<ListQuery>,
) -> Response {
    if !admin.has("user:read") {
        return forbidden();
    }
    let q: String = params.q.unwrap_or_default().trim().chars().take(100).collect();
    let page = params.page.unwrap_or(1).max(1);

    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT id, name, username, email, CAST(status AS SIGNED), photo, designation,
                COALESCE(journalist_number, '')
           FROM users WHERE 1 = 1",
    );
    push_filter(&mut qb, &q);
    qb.push(" ORDER BY id DESC LIMIT ").push_bind(PER_PAGE).push(" OFFSET ").push_bind((page - 1) * PER_PAGE);
    let users: Vec<(i32, String, String, String, i64, String, String, String)> =
        match qb.build_query_as().fetch_all(&state.db_pool).await {
            Ok(u) => u,
            Err(e) => return internal_error(e),
        };

    let mut cq = QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM users WHERE 1 = 1");
    push_filter(&mut cq, &q);
    let total: i64 = match cq.build_query_scalar().fetch_one(&state.db_pool).await {
        Ok(n) => n,
        Err(e) => return internal_error(e),
    };
    let total_pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);

    // roles of the users on this page only
    let mut roles_by_user: HashMap<i32, Vec<String>> = HashMap::new();
    if !users.is_empty() {
        let mut rq = QueryBuilder::<MySql>::new(
            "SELECT ur.user_id, r.name FROM user_roles ur JOIN roles r ON r.id = ur.role_id WHERE ur.user_id IN (",
        );
        {
            let mut sep = rq.separated(", ");
            for u in &users {
                sep.push_bind(u.0);
            }
        }
        rq.push(") ORDER BY r.name");
        let pairs: Vec<(i32, String)> = match rq.build_query_as().fetch_all(&state.db_pool).await {
            Ok(p) => p,
            Err(e) => return internal_error(e),
        };
        for (uid, role) in pairs {
            roles_by_user.entry(uid).or_default().push(role);
        }
    }

    let rows = users
        .into_iter()
        .map(|(id, name, username, email, status, photo, designation, journalist_number)| UserRow {
            id,
            initial: name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default(),
            name,
            username,
            email,
            active: status != 0,
            roles: roles_by_user.remove(&id).map(|r| r.join(", ")).unwrap_or_default(),
            photo,
            designation,
            journalist_number,
        })
        .collect();

    let layout = build_layout(&session, &admin).await;
    render(UsersListTemplate {
        layout,
        users: rows,
        can_create: admin.has("user:create"),
        can_edit: admin.has("user:update"),
        q,
        total,
        page,
        total_pages,
        has_prev: page > 1,
        has_next: page < total_pages,
        prev: page - 1,
        next: page + 1,
    })
}

// ---------- create ----------

async fn new_page(
    pool: &MySqlPool,
    session: &Session,
    admin: &CurrentAdmin,
    v: UserValues,
    selected_roles: &[i32],
    error: Option<String>,
) -> Response {
    let roles = match role_options(pool, selected_roles).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let show_press_card = roles.iter().any(|r| r.checked && r.is_journalist);
    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }
    let blood_groups = blood_options(&v.staff.blood_group);
    render(NewTemplate { layout, v, roles, can_assign: admin.has("role:assign"), show_press_card, blood_groups })
}

pub async fn new_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("user:create") {
        return forbidden();
    }
    new_page(&state.db_pool, &session, &admin, UserValues { active: true, ..Default::default() }, &[], None).await
}

#[derive(Deserialize)]
pub struct CreateUserForm {
    pub csrf_token: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub status: Option<String>, // checkbox: present when ticked
    #[serde(default)]
    pub role_ids: Vec<i32>,
    // profile / press card
    #[serde(default)]
    pub photo: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub designation: String,
    #[serde(default)]
    pub blood_group: String,
    #[serde(default)]
    pub journalist_number: String,
    #[serde(default)]
    pub card_issue_date: String,
    #[serde(default)]
    pub card_expiry_date: String,
}

fn staff_from(photo: &str, bio: &str, designation: &str, blood_group: &str, number: &str, issue: &str, expiry: &str) -> StaffFields {
    StaffFields {
        photo: photo.trim().to_string(),
        bio: bio.to_string(),
        designation: designation.to_string(),
        blood_group: blood_group.trim().to_string(),
        journalist_number: number.to_string(),
        card_issue_date: issue.trim().to_string(),
        card_expiry_date: expiry.trim().to_string(),
    }
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<CreateUserForm>,
) -> Response {
    if !admin.has("user:create") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }

    let v = UserValues {
        name: profile::clean_line(&f.name),
        email: f.email.trim().to_string(),
        phone: profile::clean_line(&f.phone),
        username: f.username.trim().to_string(),
        active: f.status.is_some(),
        staff: staff_from(&f.photo, &f.bio, &f.designation, &f.blood_group, &f.journalist_number, &f.card_issue_date, &f.card_expiry_date),
    };

    let staff = v.staff.parse();
    let error: Option<String> = if v.name.is_empty() || v.name.chars().count() > 255 {
        Some("A name is required (max 255 characters).".into())
    } else if v.username.is_empty() || v.username.chars().count() > 255 {
        Some("A username is required (max 255 characters).".into())
    } else if !profile::valid_email(&v.email) {
        Some("Enter a valid email address.".into())
    } else if v.phone.is_empty() || v.phone.chars().count() > 50 {
        Some("A phone number is required (max 50 characters).".into())
    } else if f.password.chars().count() < MIN_PASSWORD_LEN {
        Some("The password must be at least 10 characters.".into())
    } else {
        staff.as_ref().err().map(|m| m.to_string())
    };
    if let Some(msg) = error {
        return new_page(&state.db_pool, &session, &admin, v, &f.role_ids, Some(msg)).await;
    }
    let staff = match staff {
        Ok(s) => s,
        Err(_) => return forbidden(),
    };

    let hash = match security::hash_password(&f.password) {
        Ok(h) => h,
        Err(e) => return internal_error(e),
    };

    let mut tx = match state.db_pool.begin().await {
        Ok(t) => t,
        Err(e) => return internal_error(e),
    };

    let res = sqlx::query(
        "INSERT INTO users (name, email, phone, username, password, status,
                            photo, bio, designation, blood_group, journalist_number, card_issue_date, card_expiry_date)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&v.name)
    .bind(&v.email)
    .bind(&v.phone)
    .bind(&v.username)
    .bind(hash)
    .bind(v.active)
    .bind(&staff.photo)
    .bind(&staff.bio)
    .bind(&staff.designation)
    .bind(&staff.blood_group)
    .bind(&staff.journalist_number)
    .bind(&staff.card_issue_date)
    .bind(&staff.card_expiry_date)
    .execute(&mut *tx)
    .await;

    let user_id = match res {
        Ok(r) => r.last_insert_id() as i32,
        Err(e) if is_unique_violation(&e) => {
            let msg = dup_message(&e).to_string();
            return new_page(&state.db_pool, &session, &admin, v, &f.role_ids, Some(msg)).await;
        }
        Err(e) => return internal_error(e),
    };

    // Only people allowed to assign roles can set them (prevents privilege escalation).
    if admin.has("role:assign") {
        for role_id in &f.role_ids {
            if let Err(e) = sqlx::query("INSERT IGNORE INTO user_roles (user_id, role_id) VALUES (?, ?)")
                .bind(user_id)
                .bind(role_id)
                .execute(&mut *tx)
                .await
            {
                return internal_error(e);
            }
        }
    }

    if let Err(e) = tx.commit().await {
        return internal_error(e);
    }

    flash(&session, &format!("User '{}' created.", v.username)).await;
    Redirect::to("/users").into_response()
}

// ---------- edit ----------

async fn edit_page(
    pool: &MySqlPool,
    session: &Session,
    admin: &CurrentAdmin,
    id: i32,
    v: UserValues,
    selected_roles: &[i32],
    error: Option<String>,
) -> Response {
    let roles = match role_options(pool, selected_roles).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let show_press_card = roles.iter().any(|r| r.checked && r.is_journalist) || !v.staff.journalist_number.is_empty();
    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }
    let blood_groups = blood_options(&v.staff.blood_group);
    render(EditTemplate {
        layout,
        id,
        v,
        is_self: id == admin.id,
        roles,
        can_assign: admin.has("role:assign"),
        can_delete: admin.has("user:delete") && id != admin.id,
        show_press_card,
        blood_groups,
    })
}

pub async fn edit_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    if !admin.has("user:update") {
        return forbidden();
    }

    let user: Result<Option<(String, String, String, String, i64)>, _> = sqlx::query_as(
        "SELECT name, username, email, phone, CAST(status AS SIGNED) FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await;
    let (name, username, email, phone, status) = match user {
        Ok(Some(u)) => u,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    let staff = match profile::staff_fields(&state.db_pool, id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    let selected: Vec<i32> = match sqlx::query_scalar("SELECT role_id FROM user_roles WHERE user_id = ?")
        .bind(id)
        .fetch_all(&state.db_pool)
        .await
    {
        Ok(v) => v,
        Err(e) => return internal_error(e),
    };

    let v = UserValues { name, email, phone, username, active: status != 0, staff };
    edit_page(&state.db_pool, &session, &admin, id, v, &selected, None).await
}

#[derive(Deserialize)]
pub struct UpdateUserForm {
    pub csrf_token: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    pub status: Option<String>,
    pub new_password: Option<String>,
    #[serde(default)]
    pub role_ids: Vec<i32>,
    #[serde(default)]
    pub photo: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub designation: String,
    #[serde(default)]
    pub blood_group: String,
    #[serde(default)]
    pub journalist_number: String,
    #[serde(default)]
    pub card_issue_date: String,
    #[serde(default)]
    pub card_expiry_date: String,
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<UpdateUserForm>,
) -> Response {
    if !admin.has("user:update") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let is_self = id == admin.id;

    let username: String = match sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };

    let v = UserValues {
        name: profile::clean_line(&f.name),
        email: f.email.trim().to_string(),
        phone: profile::clean_line(&f.phone),
        username,
        active: f.status.is_some() || is_self,
        staff: staff_from(&f.photo, &f.bio, &f.designation, &f.blood_group, &f.journalist_number, &f.card_issue_date, &f.card_expiry_date),
    };
    let new_password = f.new_password.as_deref().map(str::trim).filter(|p| !p.is_empty());

    let staff = v.staff.parse();
    let error: Option<String> = if v.name.is_empty() || v.name.chars().count() > 255 {
        Some("A name is required (max 255 characters).".into())
    } else if !profile::valid_email(&v.email) {
        Some("Enter a valid email address.".into())
    } else if v.phone.is_empty() || v.phone.chars().count() > 50 {
        Some("A phone number is required (max 50 characters).".into())
    } else if new_password.map(|p| p.chars().count() < MIN_PASSWORD_LEN).unwrap_or(false) {
        Some("The new password must be at least 10 characters.".into())
    } else {
        staff.as_ref().err().map(|m| m.to_string())
    };
    if let Some(msg) = error {
        return edit_page(&state.db_pool, &session, &admin, id, v, &f.role_ids, Some(msg)).await;
    }
    let staff = match staff {
        Ok(s) => s,
        Err(_) => return forbidden(),
    };

    let mut tx = match state.db_pool.begin().await {
        Ok(t) => t,
        Err(e) => return internal_error(e),
    };

    let res = sqlx::query(
        "UPDATE users SET name = ?, email = ?, phone = ?, photo = ?, bio = ?, designation = ?, blood_group = ?,
                journalist_number = ?, card_issue_date = ?, card_expiry_date = ?
          WHERE id = ?",
    )
    .bind(&v.name)
    .bind(&v.email)
    .bind(&v.phone)
    .bind(&staff.photo)
    .bind(&staff.bio)
    .bind(&staff.designation)
    .bind(&staff.blood_group)
    .bind(&staff.journalist_number)
    .bind(&staff.card_issue_date)
    .bind(&staff.card_expiry_date)
    .bind(id)
    .execute(&mut *tx)
    .await;
    match res {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            let msg = dup_message(&e).to_string();
            return edit_page(&state.db_pool, &session, &admin, id, v, &f.role_ids, Some(msg)).await;
        }
        Err(e) => return internal_error(e),
    }

    // You cannot deactivate yourself or change your own roles here (avoids self-lockout).
    if !is_self {
        if let Err(e) = sqlx::query("UPDATE users SET status = ? WHERE id = ?")
            .bind(f.status.is_some())
            .bind(id)
            .execute(&mut *tx)
            .await
        {
            return internal_error(e);
        }
    }

    if let Some(p) = new_password {
        let hash = match security::hash_password(p) {
            Ok(h) => h,
            Err(e) => return internal_error(e),
        };
        if let Err(e) = sqlx::query("UPDATE users SET password = ? WHERE id = ?").bind(hash).bind(id).execute(&mut *tx).await {
            return internal_error(e);
        }
    }

    if admin.has("role:assign") && !is_self {
        if let Err(e) = sqlx::query("DELETE FROM user_roles WHERE user_id = ?").bind(id).execute(&mut *tx).await {
            return internal_error(e);
        }
        for role_id in &f.role_ids {
            if let Err(e) = sqlx::query("INSERT IGNORE INTO user_roles (user_id, role_id) VALUES (?, ?)")
                .bind(id)
                .bind(role_id)
                .execute(&mut *tx)
                .await
            {
                return internal_error(e);
            }
        }
    }

    if let Err(e) = tx.commit().await {
        return internal_error(e);
    }

    flash(&session, "User updated.").await;
    Redirect::to("/users").into_response()
}

// ---------- delete ----------

#[derive(Deserialize)]
pub struct DeleteUserForm {
    pub csrf_token: String,
}

/// Permanently deletes a user. Refused when it is your own account or when the user still
/// owns articles (deactivate such a user instead, so the articles keep their author).
pub async fn delete(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<DeleteUserForm>,
) -> Response {
    if !admin.has("user:delete") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let back = format!("/users/{id}/edit");

    if id == admin.id {
        flash(&session, "You can't delete your own account.").await;
        return Redirect::to(&back).into_response();
    }

    let username: String = match sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };

    let articles: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM articles WHERE author_id = ?")
        .bind(id)
        .fetch_one(&state.db_pool)
        .await
    {
        Ok(n) => n,
        Err(e) => return internal_error(e),
    };
    if articles > 0 {
        flash(
            &session,
            &format!("'{username}' has {articles} article(s) and can't be deleted. Deactivate the account instead."),
        )
        .await;
        return Redirect::to(&back).into_response();
    }

    // private application documents (if this user applied as a journalist), removed after the DB delete
    let files: Option<(String, String, String)> =
        match sqlx::query_as("SELECT photo_file, nid_file, resume_file FROM journalist_applications WHERE user_id = ?")
            .bind(id)
            .fetch_optional(&state.db_pool)
            .await
        {
            Ok(r) => r,
            Err(e) => return internal_error(e),
        };

    let mut tx = match state.db_pool.begin().await {
        Ok(t) => t,
        Err(e) => return internal_error(e),
    };
    if let Err(e) = sqlx::query("DELETE FROM user_roles WHERE user_id = ?").bind(id).execute(&mut *tx).await {
        return internal_error(e);
    }
    // journalist_applications rows go with the user (ON DELETE CASCADE)
    if let Err(e) = sqlx::query("DELETE FROM users WHERE id = ?").bind(id).execute(&mut *tx).await {
        if e.as_database_error().map(|d| d.is_foreign_key_violation()).unwrap_or(false) {
            flash(&session, &format!("'{username}' is still referenced by other records. Deactivate the account instead.")).await;
            return Redirect::to(&back).into_response();
        }
        return internal_error(e);
    }
    if let Err(e) = tx.commit().await {
        return internal_error(e);
    }

    if let Some((photo, nid, resume)) = files {
        for name in [photo, nid, resume] {
            // only plain file names, never a path
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
                let _ = tokio::fs::remove_file(PathBuf::from(PRIVATE_DIR).join(&name)).await;
            }
        }
    }

    flash(&session, &format!("User '{username}' deleted.")).await;
    Redirect::to("/users").into_response()
}