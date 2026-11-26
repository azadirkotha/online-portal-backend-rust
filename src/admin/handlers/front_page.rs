use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use axum_extra::extract::Form as MultiForm;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use tower_sessions::Session;

use crate::admin::csrf;
use crate::admin::helpers::{build_layout, flash, forbidden, internal_error, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::{CheckOption, Layout};
use crate::models::front_page::{self, Brief, HomeCategory, ZONES};
use crate::state::AppState;

fn ids(csv: &str) -> Vec<i32> {
    csv.split(',').filter_map(|x| x.trim().parse::<i32>().ok()).collect()
}

// ======================= main section (drag & drop) =======================

struct ZoneView {
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    capacity: usize,
    items: Vec<Brief>,
}

#[derive(Template)]
#[template(path = "admin/front_page/main.html")]
struct MainTemplate {
    layout: Layout,
    zones: Vec<ZoneView>,
    pool: Vec<Brief>,
}

pub async fn main_page(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("frontpage:manage") {
        return forbidden();
    }
    let mut zones = Vec::new();
    for z in ZONES.iter() {
        match front_page::zone_items(&state.db_pool, z.key).await {
            Ok(items) => zones.push(ZoneView { key: z.key, label: z.label, hint: z.hint, capacity: z.capacity, items }),
            Err(e) => return internal_error(e),
        }
    }
    let pool = match front_page::pool_items(&state.db_pool, 80).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };
    let layout = build_layout(&session, &admin).await;
    render(MainTemplate { layout, zones, pool })
}

#[derive(Deserialize)]
pub struct MainForm {
    csrf_token: String,
    #[serde(default)]
    lead: String,
    #[serde(default)]
    center: String,
    #[serde(default)]
    left: String,
}

pub async fn save_main(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(f): Form<MainForm>,
) -> Response {
    if !admin.has("frontpage:manage") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    let requested = [("lead", ids(&f.lead)), ("center", ids(&f.center)), ("left", ids(&f.left))];

    let all: Vec<i32> = requested.iter().flat_map(|(_, v)| v.iter().copied()).collect();
    let published = match front_page::published_among(&state.db_pool, &all).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };

    // an article can appear once; only published ones; never more than the zone holds
    let mut seen: HashSet<i32> = HashSet::new();
    let mut cleaned: Vec<(&str, Vec<i32>)> = Vec::new();
    for (key, list) in requested {
        let capacity = ZONES.iter().find(|z| z.key == key).map(|z| z.capacity).unwrap_or(0);
        let mut keep = Vec::new();
        for id in list {
            if keep.len() < capacity && published.contains(&id) && seen.insert(id) {
                keep.push(id);
            }
        }
        cleaned.push((key, keep));
    }

    match front_page::save_main(&state.db_pool, &cleaned).await {
        Ok(()) => {
            flash(&session, "Front page updated.").await;
            Redirect::to("/front-page").into_response()
        }
        Err(e) => internal_error(e),
    }
}

// ======================= which categories appear (+ order) =======================

#[derive(Template)]
#[template(path = "admin/front_page/categories.html")]
struct CategoriesTemplate {
    layout: Layout,
    items: Vec<HomeCategory>,
}

pub async fn categories_page(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("frontpage:manage") {
        return forbidden();
    }
    match front_page::home_categories(&state.db_pool).await {
        Ok(items) => {
            let layout = build_layout(&session, &admin).await;
            render(CategoriesTemplate { layout, items })
        }
        Err(e) => internal_error(e),
    }
}

#[derive(Deserialize)]
pub struct CategoriesForm {
    csrf_token: String,
    #[serde(default)]
    order: String,
    #[serde(default)]
    show: Vec<i32>,
}

pub async fn save_categories(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    MultiForm(f): MultiForm<CategoriesForm>,
) -> Response {
    if !admin.has("frontpage:manage") || !csrf::verify(&session, &f.csrf_token).await {
        return forbidden();
    }
    match front_page::save_home_categories(&state.db_pool, &ids(&f.order), &f.show).await {
        Ok(()) => {
            flash(&session, "Front page categories saved.").await;
            Redirect::to("/front-page/categories").into_response()
        }
        Err(e) => internal_error(e),
    }
}

// ======================= category lead stories =======================

struct LeadSection {
    category_id: i32,
    name: String,
    options1: Vec<CheckOption>,
    options2: Vec<CheckOption>,
}

#[derive(Template)]
#[template(path = "admin/front_page/leads.html")]
struct LeadsTemplate {
    layout: Layout,
    sections: Vec<LeadSection>,
}

pub async fn leads_page(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    if !admin.has("frontpage:manage") {
        return forbidden();
    }
    let cats = match front_page::home_categories(&state.db_pool).await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    let mut sections = Vec::new();
    for c in cats {
        let candidates = match front_page::lead_candidates(&state.db_pool, c.id, 40).await {
            Ok(v) => v,
            Err(e) => return internal_error(e),
        };
        let (l1, l2) = match front_page::current_leads(&state.db_pool, c.id).await {
            Ok(v) => v,
            Err(e) => return internal_error(e),
        };
        let mk = |selected: i32| -> Vec<CheckOption> {
            candidates
                .iter()
                .map(|(id, label)| CheckOption {
                    id: *id,
                    label: label.chars().take(90).collect(),
                    checked: *id == selected,
                })
                .collect()
        };
        sections.push(LeadSection { category_id: c.id, name: c.name, options1: mk(l1), options2: mk(l2) });
    }

    let layout = build_layout(&session, &admin).await;
    render(LeadsTemplate { layout, sections })
}

/// Field names are dynamic (`lead1_<category id>`, `lead2_<category id>`), so read raw pairs.
pub async fn save_leads(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let token = pairs.iter().find(|(k, _)| k == "csrf_token").map(|(_, v)| v.as_str()).unwrap_or("");
    if !admin.has("frontpage:manage") || !csrf::verify(&session, token).await {
        return forbidden();
    }
    let fields: HashMap<&str, i32> = pairs
        .iter()
        .filter_map(|(k, v)| v.trim().parse::<i32>().ok().map(|n| (k.as_str(), n)))
        .collect();

    let cats = match front_page::home_categories(&state.db_pool).await {
        Ok(c) => c,
        Err(e) => return internal_error(e),
    };

    for c in cats {
        let mut picks = [0i32; 2];
        for (i, key) in [format!("lead1_{}", c.id), format!("lead2_{}", c.id)].iter().enumerate() {
            if let Some(&article_id) = fields.get(key.as_str()) {
                if article_id > 0 {
                    match front_page::belongs_to_category(&state.db_pool, article_id, c.id).await {
                        Ok(true) => picks[i] = article_id,
                        Ok(false) => {}
                        Err(e) => return internal_error(e),
                    }
                }
            }
        }
        if picks[0] == picks[1] {
            picks[1] = 0; // the same story cannot lead twice
        }
        if let Err(e) = front_page::save_leads(&state.db_pool, c.id, picks[0], picks[1]).await {
            return internal_error(e);
        }
    }

    flash(&session, "Category lead stories saved.").await;
    Redirect::to("/front-page/leads").into_response()
}
