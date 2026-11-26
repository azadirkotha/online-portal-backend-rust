use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use axum_extra::extract::Form;
use serde::Deserialize;
use tower_sessions::Session;

use super::auth::CsrfOnly;
use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, not_found, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::{CheckOption, Layout};
use crate::models::menu::{self, ItemFull, Node, TreeNode, LOCATIONS};
use crate::state::AppState;

struct LocationTab {
    key: &'static str,
    label: &'static str,
    active: bool,
}

struct CheckGroup {
    name: String,
    items: Vec<CheckOption>,
}

#[derive(Template)]
#[template(path = "admin/menus/builder.html")]
struct BuilderTemplate {
    layout: Layout,
    location: String,
    location_label: &'static str,
    tabs: Vec<LocationTab>,
    nodes: Vec<Node>,
    categories: Vec<CheckOption>,
    sub_groups: Vec<CheckGroup>,
    can_write: bool,
}

#[derive(Template)]
#[template(path = "admin/menus/edit.html")]
struct EditTemplate {
    layout: Layout,
    item: ItemFull,
    location_label: &'static str,
}

fn back(location: &str) -> Response {
    Redirect::to(&format!("/menus/{location}")).into_response()
}

/// Both checks every change needs: the permission and a valid CSRF token.
async fn may_write(admin: &CurrentAdmin, session: &Session, token: &str) -> bool {
    admin.has("menu:write") && csrf::verify(session, token).await
}

pub async fn index(Extension(admin): Extension<CurrentAdmin>) -> Response {
    if !admin.has("menu:read") {
        return forbidden();
    }
    Redirect::to("/menus/header").into_response()
}

pub async fn builder(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(location): Path<String>,
) -> Response {
    if !admin.has("menu:read") {
        return forbidden();
    }
    let Some(location_label) = menu::location_label(&location) else { return not_found() };
    let pool = &state.db_pool;

    let rows = match menu::items(pool, &location).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let nodes = menu::build_tree(rows);

    let categories = match menu::available_categories(pool, &location).await {
        Ok(c) => c.into_iter().map(|(id, label)| CheckOption { id, label, checked: false }).collect(),
        Err(e) => return internal_error(e),
    };

    let subs = match menu::available_sub_categories(pool, &location).await {
        Ok(s) => s,
        Err(e) => return internal_error(e),
    };
    let mut sub_groups: Vec<CheckGroup> = Vec::new();
    let mut last_category = -1;
    for (id, category_id, name, category_name) in subs {
        if category_id != last_category {
            sub_groups.push(CheckGroup { name: category_name, items: Vec::new() });
            last_category = category_id;
        }
        if let Some(g) = sub_groups.last_mut() {
            g.items.push(CheckOption { id, label: name, checked: false });
        }
    }

    let tabs = LOCATIONS
        .iter()
        .map(|(key, label)| LocationTab { key, label, active: *key == location })
        .collect();

    let layout = build_layout(&session, &admin).await;
    render(BuilderTemplate {
        layout,
        location,
        location_label,
        tabs,
        nodes,
        categories,
        sub_groups,
        can_write: admin.has("menu:write"),
    })
}

// ---------- drag & drop order ----------

#[derive(Deserialize)]
pub struct OrderForm {
    csrf_token: String,
    #[serde(default)]
    tree: String,
}

pub async fn save_order(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(location): Path<String>,
    Form(f): Form<OrderForm>,
) -> Response {
    if !may_write(&admin, &session, &f.csrf_token).await {
        return forbidden();
    }
    if menu::location_label(&location).is_none() {
        return not_found();
    }
    let tree: Vec<TreeNode> = match serde_json::from_str(&f.tree) {
        Ok(t) => t,
        Err(_) => {
            flash(&session, "The new order could not be read. Reload the page and try again.").await;
            return back(&location);
        }
    };
    if tree.len() > 500 {
        return forbidden();
    }
    match menu::save_order(&state.db_pool, &location, &tree).await {
        Ok(()) => flash(&session, "Menu order saved.").await,
        Err(e) => return internal_error(e),
    }
    back(&location)
}

// ---------- adding items ----------

#[derive(Deserialize)]
pub struct IdsForm {
    csrf_token: String,
    #[serde(default)]
    ids: Vec<i32>,
}

pub async fn add_categories(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(location): Path<String>,
    Form(f): Form<IdsForm>,
) -> Response {
    if !may_write(&admin, &session, &f.csrf_token).await {
        return forbidden();
    }
    if menu::location_label(&location).is_none() {
        return not_found();
    }
    if f.ids.is_empty() {
        flash(&session, "Tick at least one category.").await;
        return back(&location);
    }
    match menu::add_categories(&state.db_pool, &location, &f.ids).await {
        Ok(n) => flash(&session, &format!("{n} categor{} added.", if n == 1 { "y" } else { "ies" })).await,
        Err(e) => return internal_error(e),
    }
    back(&location)
}

pub async fn add_sub_categories(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(location): Path<String>,
    Form(f): Form<IdsForm>,
) -> Response {
    if !may_write(&admin, &session, &f.csrf_token).await {
        return forbidden();
    }
    if menu::location_label(&location).is_none() {
        return not_found();
    }
    if f.ids.is_empty() {
        flash(&session, "Tick at least one sub category.").await;
        return back(&location);
    }
    match menu::add_sub_categories(&state.db_pool, &location, &f.ids).await {
        Ok(n) => flash(&session, &format!("{n} sub categor{} added.", if n == 1 { "y" } else { "ies" })).await,
        Err(e) => return internal_error(e),
    }
    back(&location)
}

#[derive(Deserialize)]
pub struct LinkForm {
    csrf_token: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    url: String,
    new_tab: Option<String>,
}

pub async fn add_link(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(location): Path<String>,
    Form(f): Form<LinkForm>,
) -> Response {
    if !may_write(&admin, &session, &f.csrf_token).await {
        return forbidden();
    }
    if menu::location_label(&location).is_none() {
        return not_found();
    }
    let label = f.label.trim();
    let url = f.url.trim();
    if label.is_empty() || label.chars().count() > 120 {
        flash(&session, "Give the link a label (max 120 characters).").await;
        return back(&location);
    }
    if !menu::valid_link(url) {
        flash(&session, "The link must start with / (a page on this site), https://, http://, mailto: or tel:, and contain no spaces.").await;
        return back(&location);
    }
    match menu::add_link(&state.db_pool, &location, label, url, f.new_tab.is_some()).await {
        Ok(()) => flash(&session, "Link added.").await,
        Err(e) => return internal_error(e),
    }
    back(&location)
}

// ---------- editing / deleting one item ----------

pub async fn edit_form(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
) -> Response {
    if !admin.has("menu:write") {
        return forbidden();
    }
    let item = match menu::find(&state.db_pool, id).await {
        Ok(Some(i)) => i,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    let location_label = menu::location_label(&item.location).unwrap_or("Menu");
    let layout = build_layout(&session, &admin).await;
    render(EditTemplate { layout, item, location_label })
}

#[derive(Deserialize)]
pub struct ItemForm {
    csrf_token: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    url: String,
    new_tab: Option<String>,
    active: Option<String>,
}

pub async fn update(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<ItemForm>,
) -> Response {
    if !may_write(&admin, &session, &f.csrf_token).await {
        return forbidden();
    }
    let item = match menu::find(&state.db_pool, id).await {
        Ok(Some(i)) => i,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    let edit_page = || Redirect::to(&format!("/menus/items/{id}/edit")).into_response();

    let label = f.label.trim();
    let url = f.url.trim();
    if label.chars().count() > 120 {
        flash(&session, "The label is too long (max 120 characters).").await;
        return edit_page();
    }
    if item.kind == "link" {
        if label.is_empty() {
            flash(&session, "A link needs a label.").await;
            return edit_page();
        }
        if !menu::valid_link(url) {
            flash(&session, "The link must start with / (a page on this site), https://, http://, mailto: or tel:, and contain no spaces.").await;
            return edit_page();
        }
    }

    match menu::update(&state.db_pool, id, label, url, f.new_tab.is_some(), f.active.is_some()).await {
        Ok(()) => flash(&session, "Menu item saved.").await,
        Err(e) => return internal_error(e),
    }
    back(&item.location)
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Path(id): Path<i32>,
    Form(f): Form<CsrfOnly>,
) -> Response {
    if !may_write(&admin, &session, &f.csrf_token).await {
        return forbidden();
    }
    let item = match menu::find(&state.db_pool, id).await {
        Ok(Some(i)) => i,
        Ok(None) => return not_found(),
        Err(e) => return internal_error(e),
    };
    match menu::delete(&state.db_pool, id).await {
        Ok(_) => flash(&session, "Removed from the menu.").await,
        Err(e) => return internal_error(e),
    }
    back(&item.location)
}
