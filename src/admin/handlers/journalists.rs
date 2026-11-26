use std::path::PathBuf;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::extract::Form;
use serde::Deserialize;
use sqlx::{MySql, MySqlPool, QueryBuilder};
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, is_unique_violation, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::Layout;
use crate::models::journalist as jm;
use crate::models::profile::{self, clean_line, clean_multiline, StaffFields, BLOOD_GROUPS, JOURNALIST_ROLE};
use crate::security;
use crate::state::AppState;

/// Permission needed to see, create, edit and review journalists. Create it in your
/// permissions table and give it to the admin role(s) that should manage journalists.
const PERM: &str = "journalist:review";
/// Must match the API controller (journalist applications' private documents).
const PRIVATE_DIR: &str = "private_uploads/applications";
/// Folder served at /uploads.
const PUBLIC_DIR: &str = "uploads";
const PER_PAGE: i64 = 15;
const MIN_PASSWORD_LEN: usize = 8;

// ---------- view models ----------

pub struct BloodOption {
    pub value: &'static str,
    pub selected: bool,
}

fn blood_options(selected: &str) -> Vec<BloodOption> {
    BLOOD_GROUPS.iter().map(|b| BloodOption { value: b, selected: *b == selected }).collect()
}

#[derive(Template)]
#[template(path = "admin/journalists/list.html")]
struct ListTemplate {
    layout: Layout,
    items: Vec<jm::ApplicationRow>,
    status: String,
    q: String,
    page: i64,
    total_pages: i64,
    has_prev: bool,
    has_next: bool,
    prev: i64,
    next: i64,
    c_pending: i64,
    c_approved: i64,
    c_rejected: i64,
    c_all: i64,
}

#[derive(Template)]
#[template(path = "admin/journalists/detail.html")]
struct DetailTemplate {
    layout: Layout,
    a: jm::ApplicationDetail,
    has_photo: bool,
    has_nid: bool,
    has_resume: bool,
}

/// Everything shown in the journalist form (also used to show it again after an error).
#[derive(Default)]
pub struct Values {
    pub name: String,
    pub username: String,
    pub email: String,
    pub phone: String,
    pub nid_number: String,
    pub occupation: String,
    pub current_platform: String,
    pub platform_phone: String,
    pub about: String,
    pub active: bool,
    /// photo, bio, designation, blood group, journalist number and card dates (stored on the user)
    pub staff: StaffFields,
}

#[derive(Template)]
#[template(path = "admin/journalists/form.html")]
struct FormTemplate {
    layout: Layout,
    is_new: bool,
    /// application id (0 for a new journalist)
    id: i32,
    /// application status ("" for a new journalist)
    status: String,
    v: Values,
    blood_groups: Vec<BloodOption>,
}

// ---------- list ----------

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
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

/// Status tab + search (name, username, email, phone, NID number, journalist number).
/// Bengali digits are matched as well as their ASCII form.
fn push_filter(qb: &mut QueryBuilder<'_, MySql>, status: &str, q: &str) {
    if status != "all" {
        qb.push(" AND a.status = ").push_bind(status.to_string());
    }
    if q.is_empty() {
        return;
    }
    let a = like(q);
    let b = like(&jm::ascii_digits(q));
    qb.push(" AND (");
    let fields = ["a.full_name", "u.username", "u.email", "a.phone", "a.nid_number", "u.journalist_number"];
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            qb.push(" OR ");
        }
        qb.push(*f).push(" LIKE ").push_bind(a.clone()).push(" OR ").push(*f).push(" LIKE ").push_bind(b.clone());
    }
    qb.push(")");
}

async fn search(pool: &MySqlPool, status: &str, q: &str, page: i64) -> Result<(Vec<jm::ApplicationRow>, i64), sqlx::Error> {
    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT a.id, a.user_id, a.full_name, u.username, u.email, a.phone, a.occupation, a.current_platform, a.status,
                DATE_FORMAT(a.created_at, '%Y-%m-%dT%H:%i:%s') AS created_at
           FROM journalist_applications a JOIN users u ON u.id = a.user_id
          WHERE 1 = 1",
    );
    push_filter(&mut qb, status, q);
    qb.push(" ORDER BY a.id DESC LIMIT ").push_bind(PER_PAGE).push(" OFFSET ").push_bind((page - 1) * PER_PAGE);
    let items = qb.build_query_as::<jm::ApplicationRow>().fetch_all(pool).await?;

    let mut cq = QueryBuilder::<MySql>::new(
        "SELECT COUNT(*) FROM journalist_applications a JOIN users u ON u.id = a.user_id WHERE 1 = 1",
    );
    push_filter(&mut cq, status, q);
    let total: i64 = cq.build_query_scalar().fetch_one(pool).await?;
    Ok((items, total))
}

pub async fn list(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Query(params): Query<ListQuery>,
) -> Response {
    if !admin.has(PERM) {
        return forbidden();
    }
    let status = params
        .status
        .filter(|s| matches!(s.as_str(), "pending" | "approved" | "rejected" | "all"))
        .unwrap_or_else(|| "pending".to_string());
    let q: String = params.q.unwrap_or_default().trim().chars().take(100).collect();
    let page = params.page.unwrap_or(1).max(1);

    let (mut items, total) = match search(&state.db_pool, &status, &q, page).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    for it in items.iter_mut() {
        it.created_at = it.created_at.replace('T', " ");
    }
    let total_pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);

    let counts: Vec<(String, i64)> =
        match sqlx::query_as("SELECT status, COUNT(*) FROM journalist_applications GROUP BY status")
            .fetch_all(&state.db_pool)
            .await
        {
            Ok(c) => c,
            Err(e) => return internal_error(e),
        };
    let count_of = |s: &str| counts.iter().find(|(k, _)| k == s).map(|(_, n)| *n).unwrap_or(0);
    let (c_pending, c_approved, c_rejected) = (count_of("pending"), count_of("approved"), count_of("rejected"));

    let layout = build_layout(&session, &admin).await;
    render(ListTemplate {
        layout,
        items,
        status,
        q,
        page,
        total_pages,
        has_prev: page > 1,
        has_next: page < total_pages,
        prev: page - 1,
        next: page + 1,
        c_pending,
        c_approved,
        c_rejected,
        c_all: c_pending + c_approved + c_rejected,
    })
}

// ---------- detail ----------

/// Journalists created by an administrator have no uploaded documents (empty file names).
async fn has_file(pool: &MySqlPool, id: i32, kind: &str) -> bool {
    matches!(jm::application_file(pool, id, kind).await, Ok(Some(n)) if !n.is_empty())
}

pub async fn detail(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    if !admin.has(PERM) {
        return forbidden();
    }
    let mut a = match jm::application(&state.db_pool, id).await {
        Ok(Some(a)) => a,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    a.created_at = a.created_at.replace('T', " ");
    a.reviewed_at = a.reviewed_at.replace('T', " ");

    let has_photo = has_file(&state.db_pool, id, "photo").await;
    let has_nid = has_file(&state.db_pool, id, "nid").await;
    let has_resume = has_file(&state.db_pool, id, "resume").await;

    let layout = build_layout(&session, &admin).await;
    render(DetailTemplate { layout, a, has_photo, has_nid, has_resume })
}

// ---------- private documents ----------

/// Streams one private document (kind = photo | nid | resume) to a reviewer.
pub async fn file(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    Path((id, kind)): Path<(i32, String)>,
) -> Response {
    if !admin.has(PERM) {
        return forbidden();
    }
    let name = match jm::application_file(&state.db_pool, id, &kind).await {
        Ok(Some(n)) => n,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    // only plain file names, never a path
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
        return not_found();
    }
    let bytes = match tokio::fs::read(PathBuf::from(PRIVATE_DIR).join(&name)).await {
        Ok(b) => b,
        Err(_) => return not_found(),
    };

    let (ctype, disposition) = match name.rsplit('.').next().unwrap_or("") {
        "jpg" => ("image/jpeg", "inline"),
        "png" => ("image/png", "inline"),
        "webp" => ("image/webp", "inline"),
        "pdf" => ("application/pdf", "inline"),
        "doc" => ("application/msword", "attachment"),
        "docx" => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "attachment"),
        _ => ("application/octet-stream", "attachment"),
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ctype),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}

// ---------- create / edit a journalist (admin) ----------

#[derive(Deserialize)]
pub struct JournalistForm {
    pub csrf_token: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String, // used when creating only
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    /// required when creating; when editing, blank keeps the current password
    #[serde(default)]
    pub password: String,
    pub status: Option<String>, // checkbox: present when ticked
    // journalist details
    #[serde(default)]
    pub nid_number: String,
    #[serde(default)]
    pub occupation: String,
    #[serde(default)]
    pub current_platform: String,
    #[serde(default)]
    pub platform_phone: String,
    #[serde(default)]
    pub about: String,
    // profile / press card (stored on the user)
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

fn values_from(f: &JournalistForm) -> Values {
    Values {
        name: clean_line(&f.name),
        username: f.username.trim().to_string(),
        email: f.email.trim().to_lowercase(),
        phone: clean_line(&f.phone),
        nid_number: jm::ascii_digits(&f.nid_number.split_whitespace().collect::<String>()),
        occupation: clean_line(&f.occupation),
        current_platform: clean_line(&f.current_platform),
        platform_phone: clean_line(&f.platform_phone),
        about: clean_multiline(&f.about),
        active: f.status.is_some(),
        staff: StaffFields {
            photo: f.photo.trim().to_string(),
            bio: f.bio.clone(),
            designation: f.designation.clone(),
            blood_group: f.blood_group.trim().to_string(),
            journalist_number: f.journalist_number.clone(),
            card_issue_date: f.card_issue_date.trim().to_string(),
            card_expiry_date: f.card_expiry_date.trim().to_string(),
        },
    }
}

/// The first problem with the form, if any. `password` is required only when creating.
fn validate(v: &Values, creating: bool, password: &str) -> Option<&'static str> {
    if !(2..=100).contains(&v.name.chars().count()) {
        return Some("Name must be 2 to 100 characters.");
    }
    if creating && !jm::valid_username(&v.username) {
        return Some("Username must be 3 to 30 characters: letters, numbers, _ . -");
    }
    if !profile::valid_email(&v.email) {
        return Some("Enter a valid email address.");
    }
    if !profile::valid_phone(&v.phone) {
        return Some("Enter a valid phone number.");
    }
    if (creating || !password.is_empty()) && !(MIN_PASSWORD_LEN..=128).contains(&password.chars().count()) {
        return Some("The password must be 8 to 128 characters.");
    }
    if !jm::valid_nid(&v.nid_number) {
        return Some("NID number must be 10, 13 or 17 digits.");
    }
    if !(2..=100).contains(&v.occupation.chars().count()) {
        return Some("Occupation is required (2 to 100 characters).");
    }
    if v.current_platform.chars().count() > 150 {
        return Some("Current organisation is too long (max 150 characters).");
    }
    if !v.platform_phone.is_empty() && !profile::valid_phone(&v.platform_phone) {
        return Some("Organisation phone number is not valid.");
    }
    if v.about.chars().count() > 1000 {
        return Some("About is too long (max 1000 characters).");
    }
    None
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

async fn form_page(
    session: &Session,
    admin: &CurrentAdmin,
    is_new: bool,
    id: i32,
    status: String,
    v: Values,
    error: Option<String>,
) -> Response {
    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }
    let blood_groups = blood_options(&v.staff.blood_group);
    render(FormTemplate { layout, is_new, id, status, v, blood_groups })
}

pub async fn new_form(
    State(_state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has(PERM) {
        return forbidden();
    }
    form_page(&session, &admin, true, 0, String::new(), Values { active: true, ..Default::default() }, None).await
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<JournalistForm>,
) -> Response {
    if !admin.has(PERM) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }

    let v = values_from(&f);
    let staff = v.staff.parse();
    let error: Option<String> = match validate(&v, true, &f.password) {
        Some(m) => Some(m.to_string()),
        None => staff.as_ref().err().map(|m| m.to_string()),
    };
    if let Some(msg) = error {
        return form_page(&session, &admin, true, 0, String::new(), v, Some(msg)).await;
    }
    let staff = match staff {
        Ok(s) => s,
        Err(_) => return forbidden(),
    };

    match jm::nid_taken(&state.db_pool, &v.nid_number).await {
        Ok(false) => {}
        Ok(true) => {
            let msg = "A journalist or application with this NID number already exists.".to_string();
            return form_page(&session, &admin, true, 0, String::new(), v, Some(msg)).await;
        }
        Err(e) => return internal_error(e),
    }

    let roles: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE name = ?")
        .bind(JOURNALIST_ROLE)
        .fetch_one(&state.db_pool)
        .await
    {
        Ok(n) => n,
        Err(e) => return internal_error(e),
    };
    if roles == 0 {
        let msg = format!("There is no role named \"{JOURNALIST_ROLE}\". Create it on the Roles page first.");
        return form_page(&session, &admin, true, 0, String::new(), v, Some(msg)).await;
    }

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
            return form_page(&session, &admin, true, 0, String::new(), v, Some(msg)).await;
        }
        Err(e) => return internal_error(e),
    };

    // An application row that is already approved: the journalist panel and the public API
    // treat "approved application" as "is a journalist". No documents were uploaded.
    if let Err(e) = sqlx::query(
        "INSERT INTO journalist_applications
            (user_id, full_name, nid_number, phone, occupation, current_platform, platform_phone, about,
             photo_file, nid_file, resume_file, status, reviewed_by, reviewed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, '', '', '', 'approved', ?, NOW())",
    )
    .bind(user_id)
    .bind(&v.name)
    .bind(&v.nid_number)
    .bind(&v.phone)
    .bind(&v.occupation)
    .bind(&v.current_platform)
    .bind(&v.platform_phone)
    .bind(&v.about)
    .bind(admin.id)
    .execute(&mut *tx)
    .await
    {
        return internal_error(e);
    }

    if let Err(e) = sqlx::query("INSERT IGNORE INTO user_roles (user_id, role_id) SELECT ?, r.id FROM roles r WHERE r.name = ?")
        .bind(user_id)
        .bind(JOURNALIST_ROLE)
        .execute(&mut *tx)
        .await
    {
        return internal_error(e);
    }

    if let Err(e) = tx.commit().await {
        return internal_error(e);
    }

    flash(&session, &format!("Journalist '{}' created.", v.username)).await;
    Redirect::to("/journalists?status=approved").into_response()
}

#[derive(sqlx::FromRow)]
struct EditRow {
    user_id: i32,
    username: String,
    name: String,
    email: String,
    phone: String,
    active: i64,
    nid_number: String,
    occupation: String,
    current_platform: String,
    platform_phone: String,
    about: String,
    status: String,
}

pub async fn edit_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    if !admin.has(PERM) {
        return forbidden();
    }

    let row: Option<EditRow> = match sqlx::query_as(
        "SELECT a.user_id, u.username, u.name, u.email, u.phone, CAST(u.status AS SIGNED) AS active,
                a.nid_number, a.occupation, a.current_platform, a.platform_phone, a.about, a.status
           FROM journalist_applications a JOIN users u ON u.id = a.user_id
          WHERE a.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let Some(r) = row else { return not_found() };

    let staff = match profile::staff_fields(&state.db_pool, r.user_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };

    let v = Values {
        name: r.name,
        username: r.username,
        email: r.email,
        phone: r.phone,
        nid_number: r.nid_number,
        occupation: r.occupation,
        current_platform: r.current_platform,
        platform_phone: r.platform_phone,
        about: r.about,
        active: r.active != 0,
        staff,
    };
    form_page(&session, &admin, false, id, r.status, v, None).await
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<JournalistForm>,
) -> Response {
    if !admin.has(PERM) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }

    let row: Option<(i32, String, String)> = match sqlx::query_as(
        "SELECT a.user_id, u.username, a.status
           FROM journalist_applications a JOIN users u ON u.id = a.user_id
          WHERE a.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let Some((user_id, username, status)) = row else { return not_found() };
    let is_approved = status == "approved";

    let mut v = values_from(&f);
    v.username = username; // the username can't be changed here
    let pw = f.password.trim();
    let new_password = if pw.is_empty() { None } else { Some(pw) };

    let staff = v.staff.parse();
    let error: Option<String> = match validate(&v, false, pw) {
        Some(m) => Some(m.to_string()),
        None => staff.as_ref().err().map(|m| m.to_string()),
    };
    if let Some(msg) = error {
        return form_page(&session, &admin, false, id, status, v, Some(msg)).await;
    }
    let staff = match staff {
        Ok(s) => s,
        Err(_) => return forbidden(),
    };

    // the same NID may not be used by another pending / approved application
    let nid_used: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM journalist_applications WHERE nid_number = ? AND id <> ? AND status IN ('pending', 'approved')",
    )
    .bind(&v.nid_number)
    .bind(id)
    .fetch_one(&state.db_pool)
    .await
    {
        Ok(n) => n,
        Err(e) => return internal_error(e),
    };
    if nid_used > 0 {
        let msg = "Another journalist or application already uses this NID number.".to_string();
        return form_page(&session, &admin, false, id, status, v, Some(msg)).await;
    }

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
    .bind(user_id)
    .execute(&mut *tx)
    .await;
    match res {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            let msg = dup_message(&e).to_string();
            return form_page(&session, &admin, false, id, status, v, Some(msg)).await;
        }
        Err(e) => return internal_error(e),
    }

    // Only an approved journalist's sign-in can be switched on / off here (a pending applicant is
    // activated by approving the application). You can't deactivate your own account.
    if is_approved {
        let active = f.status.is_some() || user_id == admin.id;
        if let Err(e) = sqlx::query("UPDATE users SET status = ? WHERE id = ?").bind(active).bind(user_id).execute(&mut *tx).await {
            return internal_error(e);
        }
    }

    if let Some(p) = new_password {
        let hash = match security::hash_password(p) {
            Ok(h) => h,
            Err(e) => return internal_error(e),
        };
        if let Err(e) = sqlx::query("UPDATE users SET password = ? WHERE id = ?").bind(hash).bind(user_id).execute(&mut *tx).await {
            return internal_error(e);
        }
    }

    if let Err(e) = sqlx::query(
        "UPDATE journalist_applications
            SET full_name = ?, nid_number = ?, phone = ?, occupation = ?, current_platform = ?, platform_phone = ?, about = ?
          WHERE id = ?",
    )
    .bind(&v.name)
    .bind(&v.nid_number)
    .bind(&v.phone)
    .bind(&v.occupation)
    .bind(&v.current_platform)
    .bind(&v.platform_phone)
    .bind(&v.about)
    .bind(id)
    .execute(&mut *tx)
    .await
    {
        return internal_error(e);
    }

    if let Err(e) = tx.commit().await {
        return internal_error(e);
    }

    flash(&session, "Journalist updated.").await;
    Redirect::to(&format!("/journalists/{id}")).into_response()
}

// ---------- approve / reject ----------

#[derive(Deserialize)]
pub struct ReviewForm {
    pub csrf_token: String,
}

#[derive(Deserialize)]
pub struct RejectForm {
    pub csrf_token: String,
    #[serde(default)]
    pub note: String,
}

/// Saves into the public uploads folder and returns the path to store, e.g. /uploads/2026/11/ab12.jpg
async fn save_public(bytes: &[u8], ext: &str) -> Result<String, std::io::Error> {
    let sub = chrono::Utc::now().format("%Y/%m").to_string();
    let dir = PathBuf::from(PUBLIC_DIR).join(&sub);
    tokio::fs::create_dir_all(&dir).await?;
    let name = format!("{:032x}.{ext}", rand::random::<u128>());
    tokio::fs::write(dir.join(&name), bytes).await?;
    Ok(format!("/{PUBLIC_DIR}/{sub}/{name}"))
}

pub async fn approve(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<ReviewForm>,
) -> Response {
    if !admin.has(PERM) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let back = format!("/journalists/{id}");

    let photo = match jm::application_file(&state.db_pool, id, "photo").await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };

    match jm::approve(&state.db_pool, id, admin.id).await {
        Ok(jm::ReviewOutcome::Done(user_id)) => {
            // The application photo becomes the journalist's public author photo (best effort).
            if let Some(name) = photo {
                if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
                    if let Ok(bytes) = tokio::fs::read(PathBuf::from(PRIVATE_DIR).join(&name)).await {
                        let ext = name.rsplit('.').next().unwrap_or("jpg");
                        match save_public(&bytes, ext).await {
                            Ok(path) => {
                                if let Err(e) = jm::set_user_photo(&state.db_pool, user_id, &path).await {
                                    eprintln!("set_user_photo failed: {e:?}");
                                }
                            }
                            Err(e) => eprintln!("could not publish the photo of user {user_id}: {e:?}"),
                        }
                    }
                }
            }
            flash(&session, "Application approved. The user is now active with the Journalist role.").await;
            Redirect::to("/journalists").into_response()
        }
        Ok(jm::ReviewOutcome::NotFound) => not_found(),
        Ok(jm::ReviewOutcome::NotPending) => {
            flash(&session, "This application was already reviewed.").await;
            Redirect::to(&back).into_response()
        }
        Ok(jm::ReviewOutcome::RoleMissing) => {
            flash(&session, "No role named \"Journalist\" exists. Create the role first, then approve again.").await;
            Redirect::to(&back).into_response()
        }
        Err(e) => internal_error(e),
    }
}

pub async fn reject(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<RejectForm>,
) -> Response {
    if !admin.has(PERM) || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let back = format!("/journalists/{id}");

    let note = clean_multiline(&f.note);
    let len = note.chars().count();
    if !(3..=500).contains(&len) {
        flash(&session, "Write a rejection reason (3 to 500 characters).").await;
        return Redirect::to(&back).into_response();
    }

    match jm::reject(&state.db_pool, id, admin.id, &note).await {
        Ok(true) => {
            flash(&session, "Application rejected.").await;
            Redirect::to("/journalists").into_response()
        }
        Ok(false) => {
            flash(&session, "The application was not found or was already reviewed.").await;
            Redirect::to(&back).into_response()
        }
        Err(e) => internal_error(e),
    }
}