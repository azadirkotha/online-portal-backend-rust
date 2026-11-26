use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::json;
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::middleware::CurrentAdmin;
use crate::state::AppState;

const MAX_BYTES: usize = 5 * 1024 * 1024;

fn fail(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// The file type is detected from the file's own bytes, never from the name the browser sends.
/// SVG is deliberately not allowed (it can carry scripts).
fn detect_extension(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("gif")
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn random_name() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(28)
        .map(|c| (c as char).to_ascii_lowercase())
        .collect()
}

/// POST /media/upload  (multipart field `file`, header `X-CSRF-Token`) -> { "location": "/uploads/2026/10/xxxx.jpg" }
pub async fn upload_image(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    // Every signed-in dashboard user may upload (profile photos, logos, article photos); the route is
    // already behind the admin session guard, the CSRF token, the 5 MB limit and the file-type check.
    let _ = &admin;
    let token = headers.get("x-csrf-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !csrf::verify(&session, token).await {
        return fail(StatusCode::FORBIDDEN, "Your session expired. Reload the page and try again.");
    }

    let mut data: Option<Vec<u8>> = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name() == Some("file") {
                    match field.bytes().await {
                        Ok(b) => {
                            data = Some(b.to_vec());
                            break;
                        }
                        Err(_) => return fail(StatusCode::BAD_REQUEST, "The upload could not be read (is it larger than 5 MB?)."),
                    }
                }
            }
            Ok(None) => break,
            Err(_) => return fail(StatusCode::BAD_REQUEST, "Invalid upload (is it larger than 5 MB?)."),
        }
    }

    let Some(bytes) = data else { return fail(StatusCode::BAD_REQUEST, "No file was sent.") };
    if bytes.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "The file is empty.");
    }
    if bytes.len() > MAX_BYTES {
        return fail(StatusCode::PAYLOAD_TOO_LARGE, "Images can be at most 5 MB.");
    }
    let Some(ext) = detect_extension(&bytes) else {
        return fail(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Only JPG, PNG, GIF and WebP images are allowed.");
    };

    let sub = chrono::Utc::now().format("%Y/%m").to_string();
    let dir = std::path::Path::new(&state.config.upload_dir).join(&sub);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        eprintln!("upload: cannot create {dir:?}: {e}");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "The server could not store the image.");
    }
    let name = format!("{}.{ext}", random_name());
    if let Err(e) = tokio::fs::write(dir.join(&name), &bytes).await {
        eprintln!("upload: cannot write file: {e}");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "The server could not store the image.");
    }

    (StatusCode::OK, Json(json!({ "location": format!("/uploads/{sub}/{name}") }))).into_response()
}
