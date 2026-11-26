use askama::Template;

/// Data every page of the dashboard needs (sidebar, CSRF token, flash message).
pub struct Layout {
    pub username: String,
    pub csrf: String,
    pub can_users: bool,
    pub can_journalists: bool,
    pub can_roles: bool,
    pub can_categories: bool,
    pub can_sub_categories: bool,
    pub can_tags: bool,
    pub can_articles: bool,
    pub can_front_page: bool,
    pub can_menus: bool,
    pub can_settings: bool,
    pub can_breaking: bool,
    /// shown in the sidebar: the person's name, photo (may be empty) and first letter
    pub display_name: String,
    pub photo: String,
    pub initial: String,
    pub flash: Option<String>,
}

/// A checkbox in a form (a role or a permission).
pub struct CheckOption {
    pub id: i32,
    pub label: String,
    pub checked: bool,
}

pub struct UserRow {
    pub id: i32,
    pub name: String,
    pub initial: String,
    pub username: String,
    pub email: String,
    pub active: bool,
    pub roles: String,
    pub photo: String,
    pub designation: String,
    pub journalist_number: String,
}

pub struct RoleRow {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub perm_count: i64,
    pub user_count: i64,
    pub locked: bool,
}

#[derive(Template)]
#[template(path = "admin/login.html")]
pub struct LoginTemplate {
    pub csrf: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub struct DashboardTemplate {
    pub layout: Layout,
    pub users: i64,
    pub roles: i64,
    pub permissions: i64,
}

#[derive(Template)]
#[template(path = "admin/users/list.html")]
pub struct UsersListTemplate {
    pub layout: Layout,
    pub users: Vec<UserRow>,
    pub can_create: bool,
    pub can_edit: bool,
    /// current search text (empty = no filter)
    pub q: String,
    pub total: i64,
    pub page: i64,
    pub total_pages: i64,
    pub has_prev: bool,
    pub has_next: bool,
    pub prev: i64,
    pub next: i64,
}

#[derive(Template)]
#[template(path = "admin/roles/list.html")]
pub struct RolesListTemplate {
    pub layout: Layout,
    pub roles: Vec<RoleRow>,
    pub can_write: bool,
}

#[derive(Template)]
#[template(path = "admin/roles/new.html")]
pub struct RoleNewTemplate {
    pub layout: Layout,
    pub perms: Vec<CheckOption>,
}

#[derive(Template)]
#[template(path = "admin/roles/edit.html")]
pub struct RoleEditTemplate {
    pub layout: Layout,
    pub id: i32,
    pub name: String,
    pub description: String,
    pub perms: Vec<CheckOption>,
}