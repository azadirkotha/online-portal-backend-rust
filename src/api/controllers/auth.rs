use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;

use crate::api::middleware::AuthenticatedUser;
use crate::models::auth::{AuthInput, AuthResponse, Claims, RegisterInput};
use crate::models::journalist;
use crate::models::permissions::effective_for_user;
use crate::security;
use crate::state::AppState;

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterInput>,
) -> impl IntoResponse {
    let password_hash = match security::hash_password(&payload.password) {
        Ok(h) => h,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to hash password"})),
            )
                .into_response()
        }
    };

    // New accounts get status = false (pending) and NO roles/permissions.
    let result = sqlx::query(
        "INSERT INTO users (name, email, phone, username, password) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(payload.name)
    .bind(payload.email)
    .bind(payload.phone)
    .bind(payload.username)
    .bind(password_hash)
    .execute(&state.db_pool)
    .await;

    match result {
        Ok(_) => (StatusCode::CREATED, Json(json!({"message": "Registration successful"}))).into_response(),
        Err(err) => {
            // NOTE: sqlx reports the MySQL SQLSTATE (23000) from `code()`, not 1062,
            // so use the driver's own helper instead of comparing code strings.
            if err.as_database_error().map(|e| e.is_unique_violation()).unwrap_or(false) {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({"error": "Username or email already taken"})),
                )
                    .into_response();
            }
            // Log details server-side only. Never return DB internals to the client.
            eprintln!("register failed: {err:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Something went wrong"})),
            )
                .into_response()
        }
    }
}

pub async fn login(State(state): State<AppState>, Json(payload): Json<AuthInput>) -> impl IntoResponse {
    let invalid = || {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid credentials"}))).into_response()
    };

    let user_row: Option<(i32, String)> =
        match sqlx::query_as("SELECT id, password FROM users WHERE username = ?")
            .bind(&payload.username)
            .fetch_optional(&state.db_pool)
            .await
        {
            Ok(row) => row,
            Err(e) => {
                eprintln!("login query failed: {e:?}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "Something went wrong"})),
                )
                    .into_response();
            }
        };

    let Some((user_id, password_hash)) = user_row else {
        security::dummy_verify(&payload.password); // equalise timing
        return invalid();
    };

    if !security::verify_password(&payload.password, &password_hash) {
        return invalid();
    }

    // NEW: journalist applicants cannot log in until an admin approves them.
    // (Checked after the password, so the status is never revealed to a wrong password.)
    match journalist::application_status(&state.db_pool, user_id).await {
        Ok(Some(s)) if s == "pending" => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "আপনার আবেদনটি এখনও পর্যালোচনাধীন। অনুমোদন হলে আপনি লগইন করতে পারবেন।",
                    "code": "application_pending"
                })),
            )
                .into_response();
        }
        Ok(Some(s)) if s == "rejected" => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "দুঃখিত, আপনার আবেদনটি অনুমোদিত হয়নি।",
                    "code": "application_rejected"
                })),
            )
                .into_response();
        }
        Ok(_) => {}
        // fail open on purpose: pending users still have no permissions, and the journalist API re-checks approval
        Err(e) => eprintln!("application status lookup failed: {e:?}"),
    }

    let permissions = match effective_for_user(&state.db_pool, user_id).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("permission lookup failed: {e:?}");
            Vec::new()
        }
    };

    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .unwrap()
        .timestamp() as usize;

    let claims = Claims { sub: user_id, permissions, exp: expiration };

    let token = match encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    ) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Token generation failed"})),
            )
                .into_response()
        }
    };

    (StatusCode::OK, Json(AuthResponse { token })).into_response()
}

pub async fn get_profile(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> impl IntoResponse {
    let user_info: Result<(String, String, String, String), sqlx::Error> =
        sqlx::query_as("SELECT name, email, phone, username FROM users WHERE id = ?")
            .bind(claims.sub)
            .fetch_one(&state.db_pool)
            .await;

    // NEW: lets the frontend know whether to show the journalist dashboard
    let is_journalist = journalist::is_approved_journalist(&state.db_pool, claims.sub).await.unwrap_or(false);

    match user_info {
        Ok((name, email, phone, username)) => (
            StatusCode::OK,
            Json(json!({
                "id": claims.sub,
                "name": name,
                "email": email,
                "phone": phone,
                "username": username,
                "journalist": is_journalist,
                "permissions": claims.permissions
            })),
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(json!({"error": "User profile not found"}))).into_response(),
    }
}