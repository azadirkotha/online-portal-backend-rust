use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use serde::Deserialize;
use sqlx::MySqlPool;
use tower_sessions::Session;

use super::auth::CsrfOnly;
use super::common::{is_fk_violation, parse_sort_order};
use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, is_unique_violation, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::{CheckOption, Layout};
use crate::models::category;
use crate::models::slug::slugify;
use crate::models::sub_category::{self, SubCategoryInput, SubCategoryRow};
use crate::state::AppState;

#[derive(Template)]
#[template(path = "admin/sub_categories/list.html")]
struct ListTemplate {
    layout: Layout,
    items: Vec<SubCategoryRow>,
    can_write: bool,
    can_delete: bool,
}

#[derive(Template)]
#[template(path = "admin/sub_categories/form.html")]
struct FormTemplate {
    layout: Layout,
    id: i32,
    is_edit: bool,
    values: SubCategoryInput,
    categories: Vec<CheckOption>, // `checked` marks the selected parent
}

#[derive(Deserialize)]
pub struct SubCategoryForm {
    csrf_token: String,
    #[serde(default)]
    category_id: i32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    sort_order: String,
    is_active: Option<String>,
}

impl SubCategoryForm {
    fn parse(&self) -> (SubCategoryInput, Option<&'static str>) {
        let name = self.name.trim().to_string();
        let slug = slugify(if self.slug.trim().is_empty() { &name } else { &self.slug });
        let description = self.description.trim().to_string();

        let error = if self.category_id <= 0 {
            Some("Choose a parent category.")
        } else if name.is_empty() || name.chars().count() > 120 {
            Some("Name is required (max 120 characters).")
        } else if slug.is_empty() || slug.chars().count() > 150 {
            Some("A slug could not be generated. Enter one manually (max 150 characters).")
        } else if description.chars().count() > 500 {
            Some("Description is too long (max 500 characters).")
        } else {
            None
        };

        let input = SubCategoryInput {
            category_id: self.category_id,
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
    pool: &MySqlPool,
    session: &Session,
    admin: &CurrentAdmin,
    id: Option<i32>,
    values: SubCategoryInput,
    error: Option<String>,
) -> Response {
    let options = match category::options(pool).await {
        Ok(o) => o,
        Err(e) => return internal_error(e),
    };
    let categories = options
        .into_iter()
        .map(|(cid, label)| CheckOption { id: cid, label, checked: cid == values.category_id })
        .collect();

    let mut layout = build_layout(session, admin).await;
    if error.is_some() {
        layout.flash = error;
    }
    render(FormTemplate { layout, id: id.unwrap_or(0), is_edit: id.is_some(), values, categories })
}

fn unique_msg() -> Option<String> {
    Some("That name or slug is already used in this category.".into())
}

pub async fn list(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("subcategory:read") {
        return forbidden();
    }
    match sub_category::list(&state.db_pool).await {
        Ok(items) => {
            let layout = build_layout(&session, &admin).await;
            render(ListTemplate {
                layout,
                items,
                can_write: admin.has("subcategory:write"),
                can_delete: admin.has("subcategory:delete"),
            })
        }
        Err(e) => internal_error(e),
    }
}

pub async fn new_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("subcategory:write") {
        return forbidden();
    }
    let values = SubCategoryInput { is_active: true, ..Default::default() };
    form_page(&state.db_pool, &session, &admin, None, values, None).await
}

pub async fn create(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<SubCategoryForm>,
) -> Response {
    if !admin.has("subcategory:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&state.db_pool, &session, &admin, None, input, Some(msg.into())).await;
    }

    match sub_category::create(&state.db_pool, &input).await {
        Ok(_) => {
            flash(&session, "Sub category created.").await;
            Redirect::to("/sub-categories").into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            form_page(&state.db_pool, &session, &admin, None, input, unique_msg()).await
        }
        Err(e) if is_fk_violation(&e) => {
            form_page(&state.db_pool, &session, &admin, None, input, Some("Choose a valid parent category.".into())).await
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
    if !admin.has("subcategory:write") {
        return forbidden();
    }
    match sub_category::find(&state.db_pool, id).await {
        Ok(Some(values)) => form_page(&state.db_pool, &session, &admin, Some(id), values, None).await,
        Ok(None) => not_found(),
        Err(e) => internal_error(e),
    }
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<SubCategoryForm>,
) -> Response {
    if !admin.has("subcategory:write") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match sub_category::find(&state.db_pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    }

    let (input, error) = f.parse();
    if let Some(msg) = error {
        return form_page(&state.db_pool, &session, &admin, Some(id), input, Some(msg.into())).await;
    }

    match sub_category::update(&state.db_pool, id, &input).await {
        Ok(()) => {
            flash(&session, "Sub category updated.").await;
            Redirect::to("/sub-categories").into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            form_page(&state.db_pool, &session, &admin, Some(id), input, unique_msg()).await
        }
        Err(e) if is_fk_violation(&e) => {
            form_page(&state.db_pool, &session, &admin, Some(id), input, Some("Choose a valid parent category.".into())).await
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
    if !admin.has("subcategory:delete") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match sub_category::delete(&state.db_pool, id).await {
        Ok(0) => not_found(),
        Ok(_) => {
            flash(&session, "Sub category deleted.").await;
            Redirect::to("/sub-categories").into_response()
        }
        Err(e) if is_fk_violation(&e) => {
            flash(&session, "This sub category is still in use and cannot be deleted.").await;
            Redirect::to("/sub-categories").into_response()
        }
        Err(e) => internal_error(e),
    }
}
