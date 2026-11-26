use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::auth::CsrfOnly;
use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, is_unique_violation, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::Layout;
use crate::models::slug::slugify;
use crate::models::tag::{self, Tag, TagInput};
use crate::state::AppState;

#[derive(Template)]
#[template(path = "admin/tags/list.html")]
struct ListTemplate {
    layout: Layout,
    tags: Vec<Tag>,
    can_write: bool,
    can_delete: bool,
}

#[derive(Template)]
#[template(path = "admin/tags/form.html")]
struct FormTemplate {
    layout: Layout,
    id: i32,
    is_edit: bool,
    values: TagInput,
}

#[derive(Deserialize)]
pub struct TagForm {
    csrf_token: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
}

impl TagForm {
    fn parse(&self) -> (TagInput, Option<&'static str>) {
        let name = self.name.trim().to_string();
        let slug = slugify(if self.slug.trim().is_empty() { &name } else { &self.slug });

        let error = if name.is_empty() || name.chars().count() > 80 {
            Some("Name is required (max 80 characters).")
        } else if slug.is_empty() || slug.chars().count() > 100 {
            Some("A slug could not be generated. Enter one manually (max 100 characters).")
        } else {
            None
        };
        (TagInput { name, slug }, error)
    }
}

async fn form_page(
    session: &Session,
    admin: &CurrentAdmin,
    id: Option<i32>,
    values: TagInput,
    error: Option<String>,
) -> Response {
    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }
    render(FormTemplate { layout, id: id.unwrap_or(0), is_edit: id.is_some(), values })
}

pub async fn list(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("tag:read") {
        return forbidden();
    }
    match tag::list(&state.db_pool).await {
        Ok(tags) => {
            let layout = build_layout(&session, &admin).await;
            render(ListTemplate {
                layout,
                tags,
                can_write: admin.has("tag:write"),
                can_delete: admin.has("tag:delete"),
            })
        }
        Err(e) => internal_error(e),
    }
}

pub async fn new_form(Extension(admin): Extension<CurrentAdmin>, session: Session) -> Response {
    if !admin.has("tag:write") {
        return forbidden();
    }
    form_page(&session, &admin, None, TagInput::default(), None).await
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<TagForm>,
) -> Response {
    if !admin.has("tag:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&session, &admin, None, input, Some(msg.into())).await;
    }

    match tag::create(&state.db_pool, &input).await {
        Ok(_) => {
            flash(&session, "Tag created.").await;
            Redirect::to("/tags").into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            form_page(&session, &admin, None, input, Some("A tag with that name or slug already exists.".into())).await
        }
        Err(e) => internal_error(e),
    }
}

pub async fn edit_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    if !admin.has("tag:write") {
        return forbidden();
    }
    match tag::find(&state.db_pool, id).await {
        Ok(Some(values)) => form_page(&session, &admin, Some(id), values, None).await,
        Ok(None) => not_found(),
        Err(e) => internal_error(e),
    }
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<TagForm>,
) -> Response {
    if !admin.has("tag:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match tag::find(&state.db_pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    }

    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&session, &admin, Some(id), input, Some(msg.into())).await;
    }

    match tag::update(&state.db_pool, id, &input).await {
        Ok(()) => {
            flash(&session, "Tag updated.").await;
            Redirect::to("/tags").into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            form_page(&session, &admin, Some(id), input, Some("A tag with that name or slug already exists.".into())).await
        }
        Err(e) => internal_error(e),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !admin.has("tag:delete") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match tag::delete(&state.db_pool, id).await {
        Ok(0) => not_found(),
        Ok(_) => {
            flash(&session, "Tag deleted.").await;
            Redirect::to("/tags").into_response()
        }
        Err(e) => internal_error(e),
    }
}
