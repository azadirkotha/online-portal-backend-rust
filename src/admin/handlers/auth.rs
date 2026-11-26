use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::helpers::{forbidden, internal_error, render};
use crate::admin::views::LoginTemplate;
use crate::models::permissions::effective_for_user;
use crate::security;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginForm {
    pub csrf_token: String,
    pub username: String,
    pub password: String,
}

pub async fn login_page(session: Session) -> Response {
    let csrf = csrf::token(&session).await;
    render(LoginTemplate { csrf, error: None })
}

pub async fn login_submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Response {
    if !csrf::verify(&session, &form.csrf_token).await {
        return forbidden();
    }

    let fail = |session: Session| async move {
        let csrf = csrf::token(&session).await;
        render(LoginTemplate {
            csrf,
            error: Some("Invalid credentials or no admin access.".into()),
        })
    };

    // Only ACTIVE accounts (status = 1) can sign in to the dashboard.
    let row: Result<Option<(i32, String)>, sqlx::Error> =
        sqlx::query_as("SELECT id, password FROM users WHERE username = ? AND status = 1")
            .bind(form.username.trim())
            .fetch_optional(&state.db_pool)
            .await;

    let (user_id, hash) = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            security::dummy_verify(&form.password);
            return fail(session).await;
        }
        Err(e) => return internal_error(e),
    };

    if !security::verify_password(&form.password, &hash) {
        return fail(session).await;
    }

    let permissions = match effective_for_user(&state.db_pool, user_id).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };
    if !permissions.iter().any(|p| p == "admin:access") {
        return fail(session).await;
    }

    // New session id on privilege change prevents session fixation.
    if let Err(e) = session.cycle_id().await {
        return internal_error(e);
    }
    if let Err(e) = session.insert("user_id", user_id).await {
        return internal_error(e);
    }
    Redirect::to("/").into_response()
}

#[derive(Deserialize)]
pub struct CsrfOnly {
    pub csrf_token: String,
}

pub async fn logout(session: Session, Form(form): Form<CsrfOnly>) -> Response {
    if !csrf::verify(&session, &form.csrf_token).await {
        return forbidden();
    }
    let _ = session.flush().await;
    Redirect::to("/login").into_response()
}
