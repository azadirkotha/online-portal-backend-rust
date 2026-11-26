use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use std::collections::HashMap;
use tower_sessions::Session;

use crate::admin::brand;
use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::Layout;
use crate::models::site_settings::{self, FIELDS, GROUPS};
use crate::state::AppState;

struct FieldView {
    key: &'static str,
    label: &'static str,
    kind: &'static str,
    input_type: &'static str,
    help: &'static str,
    max: usize,
    value: String,
}

struct GroupView {
    title: &'static str,
    hint: &'static str,
    fields: Vec<FieldView>,
}

#[derive(Template)]
#[template(path = "admin/settings/form.html")]
struct FormTemplate {
    layout: Layout,
    groups: Vec<GroupView>,
    errors: Vec<String>,
    can_write: bool,
}

fn group_views(values: &HashMap<String, String>) -> Vec<GroupView> {
    GROUPS
        .iter()
        .map(|(key, title, hint)| GroupView {
            title,
            hint,
            fields: FIELDS
                .iter()
                .filter(|f| f.group == *key)
                .map(|f| FieldView {
                    key: f.key,
                    label: f.label,
                    kind: f.kind.as_str(),
                    input_type: f.kind.input_type(),
                    help: f.help,
                    max: f.max,
                    value: values.get(f.key).cloned().unwrap_or_default(),
                })
                .collect(),
        })
        .collect()
}

async fn page(session: &Session, admin: &CurrentAdmin, values: &HashMap<String, String>, errors: Vec<String>) -> Response {
    let layout = build_layout(session, admin).await;
    render(FormTemplate { layout, groups: group_views(values), errors, can_write: admin.has("settings:write") })
}

pub async fn show(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("settings:read") {
        return forbidden();
    }
    match site_settings::load(&state.db_pool).await {
        Ok(values) => page(&session, &admin, &values, Vec::new()).await,
        Err(e) => internal_error(e),
    }
}

/// Field names are the setting keys, so the form is read as raw pairs.
pub async fn save(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let token = pairs.iter().find(|(k, _)| k == "csrf_token").map(|(_, v)| v.as_str()).unwrap_or("");
    if !admin.has("settings:write") || !csrf::verify(&session, token).await {
        return forbidden();
    }
    let submitted: HashMap<&str, &str> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let mut cleaned: Vec<(&str, String)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for f in FIELDS.iter() {
        // a setting that was not part of the form is left as it is
        let Some(raw) = submitted.get(f.key) else { continue };
        match site_settings::clean(f, raw) {
            Ok(v) => cleaned.push((f.key, v)),
            Err(msg) => errors.push(format!("{} {}", f.label, msg)),
        }
    }

    if !errors.is_empty() {
        let entered: HashMap<String, String> =
            submitted.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        return page(&session, &admin, &entered, errors).await;
    }

    match site_settings::save(&state.db_pool, &cleaned).await {
        Ok(()) => {
            // the sidebar and the sign-in page pick up a new name / logo straight away
            brand::refresh(&state.db_pool).await;
            flash(&session, "Site settings saved.").await;
            Redirect::to("/settings").into_response()
        }
        Err(e) => internal_error(e),
    }
}