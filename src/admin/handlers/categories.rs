use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::auth::CsrfOnly;
use super::common::{is_fk_violation, parse_sort_order};
use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, is_unique_violation, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::Layout;
use crate::models::category::{self, CategoryInput, CategoryRow};
use crate::models::slug::slugify;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "admin/categories/list.html")]
struct ListTemplate {
    layout: Layout,
    categories: Vec<CategoryRow>,
    can_write: bool,
    can_delete: bool,
}

#[derive(Template)]
#[template(path = "admin/categories/form.html")]
struct FormTemplate {
    layout: Layout,
    id: i32,
    is_edit: bool,
    values: CategoryInput,
}

#[derive(Deserialize)]
pub struct CategoryForm {
    csrf_token: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    sort_order: String,
    is_active: Option<String>, // checkbox
}

impl CategoryForm {
    /// Always returns the cleaned values (so the form can be re-shown), plus an error if invalid.
    fn parse(&self) -> (CategoryInput, Option<&'static str>) {
        let name = self.name.trim().to_string();
        let slug = slugify(if self.slug.trim().is_empty() { &name } else { &self.slug });
        let description = self.description.trim().to_string();

        let error = if name.is_empty() || name.chars().count() > 120 {
            Some("Name is required (max 120 characters).")
        } else if slug.is_empty() || slug.chars().count() > 150 {
            Some("A slug could not be generated. Enter one manually (max 150 characters).")
        } else if description.chars().count() > 500 {
            Some("Description is too long (max 500 characters).")
        } else {
            None
        };

        let input = CategoryInput {
            name,
            slug,
            description,
            sort_order: parse_sort_order(&self.sort_order),
            is_active: self.is_active.is_some(),
        };
        (input, error)
    }
}

async fn form_page(
    session: &Session,
    admin: &CurrentAdmin,
    id: Option<i32>,
    values: CategoryInput,
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
    if !admin.has("category:read") {
        return forbidden();
    }
    match category::list(&state.db_pool).await {
        Ok(categories) => {
            let layout = build_layout(&session, &admin).await;
            render(ListTemplate {
                layout,
                categories,
                can_write: admin.has("category:write"),
                can_delete: admin.has("category:delete"),
            })
        }
        Err(e) => internal_error(e),
    }
}

pub async fn new_form(Extension(admin): Extension<CurrentAdmin>, session: Session) -> Response {
    if !admin.has("category:write") {
        return forbidden();
    }
    let values = CategoryInput { is_active: true, ..Default::default() };
    form_page(&session, &admin, None, values, None).await
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<CategoryForm>,
) -> Response {
    if !admin.has("category:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&session, &admin, None, input, Some(msg.into())).await;
    }

    match category::create(&state.db_pool, &input).await {
        Ok(_) => {
            flash(&session, "Category created.").await;
            Redirect::to("/categories").into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            form_page(&session, &admin, None, input, Some("A category with that name or slug already exists.".into())).await
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
    if !admin.has("category:write") {
        return forbidden();
    }
    match category::find(&state.db_pool, id).await {
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
    Form(f): Form<CategoryForm>,
) -> Response {
    if !admin.has("category:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match category::find(&state.db_pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    }

    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&session, &admin, Some(id), input, Some(msg.into())).await;
    }

    match category::update(&state.db_pool, id, &input).await {
        Ok(()) => {
            flash(&session, "Category updated.").await;
            Redirect::to("/categories").into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            form_page(&session, &admin, Some(id), input, Some("A category with that name or slug already exists.".into())).await
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
    if !admin.has("category:delete") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match category::delete(&state.db_pool, id).await {
        Ok(0) => not_found(),
        Ok(_) => {
            flash(&session, "Category deleted.").await;
            Redirect::to("/categories").into_response()
        }
        Err(e) if is_fk_violation(&e) => {
            flash(&session, "This category still has sub categories or articles. Move or delete them first.").await;
            Redirect::to("/categories").into_response()
        }
        Err(e) => internal_error(e),
    }
}
