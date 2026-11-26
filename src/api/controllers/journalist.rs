use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use crate::models::profile::{clean_line, clean_multiline};

use crate::api::middleware::AuthenticatedUser;
use crate::models::auth::Claims;
use crate::models::journalist as jm;
// use crate::models::profile::clean_multiline;
use crate::security;
use crate::state::AppState;

/// Private documents (NID copy, resume, application photo). NOT served publicly.
const PRIVATE_DIR: &str = "private_uploads/applications";
/// Folder your server exposes at /uploads (must match your existing uploader / ServeDir).
const PUBLIC_DIR: &str = "uploads";
/// Permission an admin needs to review applications (create it and give it to your admin role).
const REVIEW_PERMISSION: &str = "journalist_applications.manage";

const MAX_PHOTO: usize = 3 * 1024 * 1024;
const MAX_DOC: usize = 5 * 1024 * 1024;

// ---------- helpers ----------

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn bad(msg: &str) -> Response {
    err(StatusCode::UNPROCESSABLE_ENTITY, msg)
}

fn server_err<E: std::fmt::Debug>(e: E) -> Response {
    eprintln!("journalist: {e:?}");
    let msg = "কিছু একটা সমস্যা হয়েছে। আবার চেষ্টা করুন।";
    // the technical detail is only included in debug builds (cargo run), never in release
    let body = if cfg!(debug_assertions) {
        json!({ "error": msg, "detail": format!("{e:?}") })
    } else {
        json!({ "error": msg })
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

/// File type from the first bytes (the client's file name / content type are never trusted).
fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if b.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("webp")
    } else if b.starts_with(b"%PDF-") {
        Some("pdf")
    } else if b.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        Some("docx")
    } else if b.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        Some("doc")
    } else {
        None
    }
}

fn checked(bytes: &[u8], max: usize, allowed: &[&str], what: &str) -> Result<&'static str, Response> {
    if bytes.len() > max {
        return Err(bad(&format!("{what} অনেক বড় (সর্বোচ্চ {} এমবি)।", max / 1024 / 1024)));
    }
    match sniff(bytes) {
        Some(ext) if allowed.contains(&ext) => Ok(ext),
        _ => Err(bad(&format!("{what}-এর ফাইলের ধরন সঠিক নয়।"))),
    }
}

async fn save_private(bytes: &[u8], ext: &str) -> Result<String, Response> {
    tokio::fs::create_dir_all(PRIVATE_DIR).await.map_err(server_err)?;
    let name = format!("{:032x}.{ext}", rand::random::<u128>());
    tokio::fs::write(PathBuf::from(PRIVATE_DIR).join(&name), bytes).await.map_err(server_err)?;
    Ok(name)
}

async fn remove_private(names: &[String]) {
    for n in names {
        let _ = tokio::fs::remove_file(PathBuf::from(PRIVATE_DIR).join(n)).await;
    }
}

/// Saves into the public uploads folder and returns the path to store, e.g. /uploads/2026/11/ab12.jpg
async fn save_public(bytes: &[u8], ext: &str) -> Result<String, Response> {
    let sub = chrono::Utc::now().format("%Y/%m").to_string();
    let dir = PathBuf::from(PUBLIC_DIR).join(&sub);
    tokio::fs::create_dir_all(&dir).await.map_err(server_err)?;
    let name = format!("{:032x}.{ext}", rand::random::<u128>());
    tokio::fs::write(dir.join(&name), bytes).await.map_err(server_err)?;
    Ok(format!("/{PUBLIC_DIR}/{sub}/{name}"))
}

// ---------- public: apply ----------

pub async fn apply(State(state): State<AppState>, mut mp: Multipart) -> Result<Response, Response> {
    let mut f = jm::ApplyFields::default();
    let (mut photo, mut nid_copy, mut resume): (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>) = (None, None, None);

    while let Some(field) = mp.next_field().await.map_err(|_| bad("ফর্মের তথ্য পড়া যায়নি।"))? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "photo" | "nid_copy" | "resume" => {
                let bytes = field.bytes().await.map_err(|_| bad("ফাইল পড়া যায়নি অথবা ফাইলটি অনেক বড়।"))?;
                if bytes.is_empty() {
                    continue;
                }
                match name.as_str() {
                    "photo" => photo = Some(bytes.to_vec()),
                    "nid_copy" => nid_copy = Some(bytes.to_vec()),
                    _ => resume = Some(bytes.to_vec()),
                }
            }
            _ => {
                let text = field.text().await.map_err(|_| bad("ফর্মের তথ্য পড়া যায়নি।"))?;
                match name.as_str() {
                    "name" => f.name = text,
                    "email" => f.email = text,
                    "phone" => f.phone = text,
                    "username" => f.username = text,
                    "password" => f.password = text,
                    "nid_number" => f.nid_number = text,
                    "occupation" => f.occupation = text,
                    "current_platform" => f.current_platform = text,
                    "platform_phone" => f.platform_phone = text,
                    "about" => f.about = text,
                    _ => {}
                }
            }
        }
    }

    let f = f.clean().map_err(bad)?;
    let photo = photo.ok_or_else(|| bad("আপনার ছবি আপলোড করুন।"))?;
    let nid_copy = nid_copy.ok_or_else(|| bad("NID-এর কপি আপলোড করুন।"))?;
    let resume = resume.ok_or_else(|| bad("রেজুমে (CV) আপলোড করুন।"))?;
    let photo_ext = checked(&photo, MAX_PHOTO, &["jpg", "png", "webp"], "ছবি")?;
    let nid_ext = checked(&nid_copy, MAX_DOC, &["jpg", "png", "pdf"], "NID-এর কপি")?;
    let resume_ext = checked(&resume, MAX_DOC, &["pdf", "doc", "docx"], "রেজুমে")?;

    if jm::nid_taken(&state.db_pool, &f.nid_number).await.map_err(server_err)? {
        return Err(err(StatusCode::CONFLICT, "এই NID নম্বর দিয়ে ইতিমধ্যে আবেদন করা হয়েছে।"));
    }
    let hash = security::hash_password(&f.password)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "কিছু একটা সমস্যা হয়েছে। আবার চেষ্টা করুন।"))?;

    let mut saved: Vec<String> = Vec::new();
    for (bytes, ext) in [(&photo, photo_ext), (&nid_copy, nid_ext), (&resume, resume_ext)] {
        match save_private(bytes, ext).await {
            Ok(n) => saved.push(n),
            Err(e) => {
                remove_private(&saved).await;
                return Err(e);
            }
        }
    }

    match jm::create_application(&state.db_pool, &f, &hash, &saved[0], &saved[1], &saved[2]).await {
        Ok(_) => Ok((
            StatusCode::CREATED,
            Json(json!({ "message": "আপনার আবেদন জমা হয়েছে। যাচাইয়ের পর অনুমোদন হলে আপনি লগইন করতে পারবেন।" })),
        )
            .into_response()),
        Err(e) => {
            remove_private(&saved).await;
            match e {
                jm::ApplyError::Duplicate => Err(err(StatusCode::CONFLICT, "এই ইউজারনেম বা ইমেইল আগে থেকেই ব্যবহার করা হয়েছে।")),
                jm::ApplyError::Db(e) => Err(server_err(e)),
            }
        }
    }
}

// ---------- public: verify a journalist ----------

#[derive(Debug, Deserialize)]
pub struct VerifyQuery {
    pub number: Option<String>,
}

pub async fn verify(State(state): State<AppState>, Query(q): Query<VerifyQuery>) -> Result<Response, Response> {
    let number = clean_line(&q.number.unwrap_or_default());
    if number.is_empty() || number.chars().count() > 50 {
        return Err(bad("সাংবাদিক আইডি নম্বর লিখুন।"));
    }
    let base = state.config.public_base_url.to_string();
    match jm::verify_journalist(&state.db_pool, &base, &number).await.map_err(server_err)? {
        Some(j) => Ok(Json(j).into_response()),
        None => Err(err(StatusCode::NOT_FOUND, "এই আইডি নম্বরে কোনো সাংবাদিক পাওয়া যায়নি।")),
    }
}

// ---------- approved journalists ----------

async fn require_journalist(state: &AppState, claims: &Claims) -> Result<(), Response> {
    match jm::is_approved_journalist(&state.db_pool, claims.sub).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(err(StatusCode::FORBIDDEN, "এই সুবিধা শুধু অনুমোদিত সাংবাদিকদের জন্য।")),
        Err(e) => Err(server_err(e)),
    }
}

pub async fn categories(State(state): State<AppState>, AuthenticatedUser(claims): AuthenticatedUser) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    let list = jm::all_categories(&state.db_pool).await.map_err(server_err)?;
    Ok(Json(list).into_response())
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

pub async fn list_articles(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    let status = q.status.filter(|s| matches!(s.as_str(), "draft" | "published"));
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(10).clamp(1, 50);
    let data = jm::own_articles(&state.db_pool, claims.sub, status.as_deref(), page, per_page).await.map_err(server_err)?;
    Ok(Json(data).into_response())
}

pub async fn get_article(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<i32>,
) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    let (a, category_ids) = jm::own_article(&state.db_pool, id, claims.sub)
        .await
        .map_err(server_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "লেখাটি পাওয়া যায়নি।"))?;

    let base = state.config.public_base_url.to_string();
    let image_url = if a.featured_image.starts_with('/') { format!("{base}{}", a.featured_image) } else { a.featured_image.clone() };
    // the editor needs absolute image URLs to show them
    let body = a.body.replace("src=\"/uploads/", &format!("src=\"{base}/uploads/"));

    Ok(Json(json!({
        "id": a.id,
        "title": a.title,
        "slug": a.slug,
        "summary": a.summary,
        "body": body,
        "kind": a.kind,
        "status": a.status,
        "featured_image": a.featured_image,
        "featured_image_url": image_url,
        "featured_image_caption": a.featured_image_caption,
        "news_source": a.news_source,
        "category_ids": category_ids,
    }))
    .into_response())
}

pub async fn create_article(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(input): Json<jm::ArticleInput>,
) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    let base = state.config.public_base_url.to_string();
    let a = input.clean(&base).map_err(bad)?;
    let (id, slug) = jm::create_article(&state.db_pool, claims.sub, &a)
        .await
        .map_err(server_err)?
        .ok_or_else(|| bad("সঠিক বিভাগ বেছে নিন।"))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "slug": slug }))).into_response())
}

pub async fn update_article(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<i32>,
    Json(input): Json<jm::ArticleInput>,
) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    match jm::own_article_status(&state.db_pool, id, claims.sub).await.map_err(server_err)? {
        None => return Err(err(StatusCode::NOT_FOUND, "লেখাটি পাওয়া যায়নি।")),
        Some(s) if s == "published" => {
            return Err(err(StatusCode::FORBIDDEN, "প্রকাশিত লেখা এখান থেকে সম্পাদনা করা যায় না। সম্পাদকের সাথে যোগাযোগ করুন।"))
        }
        Some(_) => {}
    }
    let base = state.config.public_base_url.to_string();
    let a = input.clean(&base).map_err(bad)?;
    if !jm::update_article(&state.db_pool, id, claims.sub, &a).await.map_err(server_err)? {
        return Err(bad("সঠিক বিভাগ বেছে নিন।"));
    }
    Ok(Json(json!({ "message": "সংরক্ষণ করা হয়েছে।" })).into_response())
}

pub async fn delete_article(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<i32>,
) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    match jm::own_article_status(&state.db_pool, id, claims.sub).await.map_err(server_err)? {
        None => return Err(err(StatusCode::NOT_FOUND, "লেখাটি পাওয়া যায়নি।")),
        Some(s) if s == "published" => return Err(err(StatusCode::FORBIDDEN, "প্রকাশিত লেখা মোছা যায় না।")),
        Some(_) => {}
    }
    jm::delete_article(&state.db_pool, id, claims.sub).await.map_err(server_err)?;
    Ok(Json(json!({ "message": "লেখাটি মুছে ফেলা হয়েছে।" })).into_response())
}

/// Featured-image upload for an article (public folder, images only).
pub async fn upload_image(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    mut mp: Multipart,
) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    while let Some(field) = mp.next_field().await.map_err(|_| bad("ফাইল পড়া যায়নি।"))? {
        if field.name() != Some("file") {
            continue;
        }
        let bytes = field.bytes().await.map_err(|_| bad("ফাইল পড়া যায়নি অথবা ফাইলটি অনেক বড়।"))?;
        let ext = checked(&bytes, MAX_PHOTO, &["jpg", "png", "webp"], "ছবি")?;
        let path = save_public(&bytes, ext).await?;
        let url = format!("{}{}", state.config.public_base_url, path);
        return Ok((StatusCode::CREATED, Json(json!({ "path": path, "url": url }))).into_response());
    }
    Err(bad("কোনো ছবি পাওয়া যায়নি।"))
}

pub async fn me(State(state): State<AppState>, AuthenticatedUser(claims): AuthenticatedUser) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;
    let base = state.config.public_base_url.to_string();
    let info = jm::my_info(&state.db_pool, &base, claims.sub)
        .await
        .map_err(server_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "প্রোফাইল পাওয়া যায়নি।"))?;
    Ok(Json(info).into_response())
}

// ---------- change password (journalists only, from their own panel) ----------

#[derive(Debug, Deserialize)]
pub struct PasswordInput {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(input): Json<PasswordInput>,
) -> Result<Response, Response> {
    require_journalist(&state, &claims).await?;

    if !(8..=128).contains(&input.new_password.chars().count()) {
        return Err(bad("নতুন পাসওয়ার্ড কমপক্ষে ৮ অক্ষরের হতে হবে।"));
    }
    if input.new_password == input.current_password {
        return Err(bad("নতুন পাসওয়ার্ড বর্তমান পাসওয়ার্ডের থেকে আলাদা হতে হবে।"));
    }

    let hash = jm::password_hash(&state.db_pool, claims.sub)
        .await
        .map_err(server_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "প্রোফাইল পাওয়া যায়নি।"))?;

    // 422 (not 401) on purpose: a wrong current password must not look like an expired login.
    // NOTE: adjust this call to the verify function in your `security` module.
    if !security::verify_password(&input.current_password, &hash) {
        return Err(bad("বর্তমান পাসওয়ার্ড সঠিক নয়।"));
    }

    let new_hash = security::hash_password(&input.new_password)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "কিছু একটা সমস্যা হয়েছে। আবার চেষ্টা করুন।"))?;
    jm::set_password(&state.db_pool, claims.sub, &new_hash).await.map_err(server_err)?;

    Ok(Json(json!({ "message": "পাসওয়ার্ড পরিবর্তন করা হয়েছে।" })).into_response())
}

// ---------- admin review ----------

fn require_admin(claims: &Claims) -> Result<(), Response> {
    if claims.permissions.iter().any(|p| p == REVIEW_PERMISSION) {
        Ok(())
    } else {
        Err(err(StatusCode::FORBIDDEN, "এই কাজের অনুমতি আপনার নেই।"))
    }
}

pub async fn admin_list(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, Response> {
    require_admin(&claims)?;
    let status = q.status.filter(|s| matches!(s.as_str(), "pending" | "approved" | "rejected" | "all")).unwrap_or_else(|| "pending".into());
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(15).clamp(1, 50);
    let data = jm::applications(&state.db_pool, &status, page, per_page).await.map_err(server_err)?;
    Ok(Json(data).into_response())
}

pub async fn admin_detail(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<i32>,
) -> Result<Response, Response> {
    require_admin(&claims)?;
    let a = jm::application(&state.db_pool, id)
        .await
        .map_err(server_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "আবেদনটি পাওয়া যায়নি।"))?;
    Ok(Json(a).into_response())
}

/// Streams one private document (kind = photo | nid | resume) to an admin.
pub async fn admin_file(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((id, kind)): Path<(i32, String)>,
) -> Result<Response, Response> {
    require_admin(&claims)?;
    let not_found = || err(StatusCode::NOT_FOUND, "ফাইল পাওয়া যায়নি।");
    let name = jm::application_file(&state.db_pool, id, &kind).await.map_err(server_err)?.ok_or_else(not_found)?;
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
        return Err(not_found());
    }
    let bytes = tokio::fs::read(PathBuf::from(PRIVATE_DIR).join(&name)).await.map_err(|_| not_found())?;

    let (ctype, disposition) = match name.rsplit('.').next().unwrap_or("") {
        "jpg" => ("image/jpeg", "inline"),
        "png" => ("image/png", "inline"),
        "webp" => ("image/webp", "inline"),
        "pdf" => ("application/pdf", "inline"),
        "doc" => ("application/msword", "attachment"),
        "docx" => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "attachment"),
        _ => ("application/octet-stream", "attachment"),
    };
    Ok((
        [
            (header::CONTENT_TYPE, ctype),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

pub async fn admin_approve(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<i32>,
) -> Result<Response, Response> {
    require_admin(&claims)?;
    let photo = jm::application_file(&state.db_pool, id, "photo").await.map_err(server_err)?;

    match jm::approve(&state.db_pool, id, claims.sub).await.map_err(server_err)? {
        jm::ReviewOutcome::Done(user_id) => {
            // The application photo becomes the journalist's public author photo (best effort).
            if let Some(name) = photo {
                if let Ok(bytes) = tokio::fs::read(PathBuf::from(PRIVATE_DIR).join(&name)).await {
                    let ext = name.rsplit('.').next().unwrap_or("jpg");
                    match save_public(&bytes, ext).await {
                        Ok(path) => {
                            if let Err(e) = jm::set_user_photo(&state.db_pool, user_id, &path).await {
                                eprintln!("set_user_photo failed: {e:?}");
                            }
                        }
                        Err(_) => eprintln!("could not publish the photo of user {user_id}"),
                    }
                }
            }
            Ok(Json(json!({ "message": "আবেদন অনুমোদন করা হয়েছে।" })).into_response())
        }
        jm::ReviewOutcome::NotFound => Err(err(StatusCode::NOT_FOUND, "আবেদনটি পাওয়া যায়নি।")),
        jm::ReviewOutcome::NotPending => Err(err(StatusCode::CONFLICT, "এই আবেদনটি আগেই পর্যালোচনা করা হয়েছে।")),
        jm::ReviewOutcome::RoleMissing => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "\"Journalist\" নামের রোল পাওয়া যায়নি। আগে রোলটি তৈরি করুন।")),
    }
}

#[derive(Debug, Deserialize)]
pub struct RejectInput {
    pub note: String,
}

pub async fn admin_reject(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<i32>,
    Json(input): Json<RejectInput>,
) -> Result<Response, Response> {
    require_admin(&claims)?;
    let note = clean_multiline(&input.note);
    if note.chars().count() < 3 || note.chars().count() > 500 {
        return Err(bad("বাতিলের কারণ ৩ থেকে ৫০০ অক্ষরের মধ্যে লিখুন।"));
    }
    if !jm::reject(&state.db_pool, id, claims.sub, &note).await.map_err(server_err)? {
        return Err(err(StatusCode::CONFLICT, "আবেদনটি পাওয়া যায়নি অথবা আগেই পর্যালোচনা করা হয়েছে।"));
    }
    Ok(Json(json!({ "message": "আবেদন বাতিল করা হয়েছে।" })).into_response())
}