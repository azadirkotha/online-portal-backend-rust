use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::extract::Form;
use serde::Deserialize;
use sqlx::MySqlPool;
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, is_unique_violation, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::Layout;
use crate::models::profile::{self, ProfileFields};
use crate::security;
use crate::state::AppState;

const MIN_PASSWORD_LEN: usize = 10;

/// Press-card details are read-only here: they are issued and changed by an administrator.
struct CardView {
    journalist_number: String,
    designation: String,
    blood_group: String,
    issued: String,
    expiry: String,
    expired: bool,
}

#[derive(Template)]
#[template(path = "admin/account/profile.html")]
struct ProfileTemplate {
    layout: Layout,
    username: String,
    roles: Vec<String>,
    designation: String,
    v: ProfileFields,
    card: Option<CardView>,
}

async fn page(
    pool: &MySqlPool,
    session: &Session,
    admin: &CurrentAdmin,
    v: ProfileFields,
    error: Option<String>,
) -> Response {
    let roles: Vec<String> = match sqlx::query_scalar(
        "SELECT r.name FROM user_roles ur JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = ? ORDER BY r.name",
    )
    .bind(admin.id)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let staff = match profile::staff_fields(pool, admin.id).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };

    let card = (!staff.journalist_number.is_empty()).then(|| CardView {
        journalist_number: staff.journalist_number.clone(),
        designation: staff.designation.clone(),
        blood_group: staff.blood_group.clone(),
        issued: staff.card_issue_date.clone(),
        expired: profile::card_expired(&staff.card_expiry_date),
        expiry: staff.card_expiry_date.clone(),
    });

    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }
    render(ProfileTemplate { layout, username: admin.username.clone(), roles, designation: staff.designation, v, card })
}

pub async fn show(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    match profile::profile(&state.db_pool, admin.id).await {
        Ok(Some(v)) => page(&state.db_pool, &session, &admin, v, None).await,
        Ok(None) => not_found(),
        Err(e) => internal_error(e),
    }
}

#[derive(Deserialize)]
pub struct ProfileForm {
    csrf_token: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    photo: String,
    #[serde(default)]
    bio: String,
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<ProfileForm>,
) -> Response {
    if !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }

    let v = ProfileFields {
        name: profile::clean_line(&f.name),
        email: f.email.trim().to_string(),
        phone: profile::clean_line(&f.phone),
        photo: f.photo.trim().to_string(),
        bio: profile::clean_multiline(&f.bio),
    };

    let error = if v.name.is_empty() || v.name.chars().count() > 255 {
        Some("Your name is required (max 255 characters).")
    } else if !profile::valid_email(&v.email) {
        Some("Enter a valid email address.")
    } else if v.phone.is_empty() || v.phone.chars().count() > 50 {
        Some("Your phone number is required (max 50 characters).")
    } else if !v.photo.is_empty() && !profile::valid_upload_path(&v.photo) {
        Some("The photo must be uploaded through the dashboard.")
    } else if v.bio.chars().count() > 500 {
        Some("The short bio is too long (max 500 characters).")
    } else {
        None
    };
    if let Some(msg) = error {
        return page(&state.db_pool, &session, &admin, v, Some(msg.into())).await;
    }

    match profile::update_profile(&state.db_pool, admin.id, &v).await {
        Ok(()) => {
            flash(&session, "Your profile has been updated.").await;
            Redirect::to("/account").into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            page(&state.db_pool, &session, &admin, v, Some("That email address is already used by another account.".into())).await
        }
        Err(e) => internal_error(e),
    }
}

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf_token: String,
    #[serde(default)]
    current_password: String,
    #[serde(default)]
    new_password: String,
    #[serde(default)]
    confirm_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<PasswordForm>,
) -> Response {
    if !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let back = || Redirect::to("/account").into_response();

    let stored = match profile::password_hash(&state.db_pool, admin.id).await {
        Ok(Some(h)) => h,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    if !security::verify_password(&f.current_password, &stored) {
        flash(&session, "Your current password is not correct.").await;
        return back();
    }
    if f.new_password.chars().count() < MIN_PASSWORD_LEN {
        flash(&session, "The new password must be at least 10 characters.").await;
        return back();
    }
    if f.new_password != f.confirm_password {
        flash(&session, "The new password and its confirmation do not match.").await;
        return back();
    }
    if f.new_password == f.current_password {
        flash(&session, "Choose a password you have not used before.").await;
        return back();
    }

    let hash = match security::hash_password(&f.new_password) {
        Ok(h) => h,
        Err(e) => return internal_error(e),
    };
    if let Err(e) = profile::set_password(&state.db_pool, admin.id, &hash).await {
        return internal_error(e);
    }
    // a fresh session id after a credential change
    let _ = session.cycle_id().await;
    flash(&session, "Your password has been changed.").await;
    back()
}
