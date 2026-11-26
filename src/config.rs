use std::env;

#[derive(Clone)]
pub struct AdminSeed {
    pub username: String,
    pub password: String,
    pub name: String,
    pub email: String,
    pub phone: String,
}

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    /// Bind address of the public JSON API.
    pub server_port: String,
    /// Bind address of the admin dashboard (separate listener).
    pub admin_bind: String,
    pub jwt_secret: String,
    pub frontend_origin: String,
    pub cookie_secure: bool,
    /// Where uploaded images are stored on disk (served at /uploads).
    pub upload_dir: String,
    /// Origin of the public API, used to turn /uploads/... into absolute URLs in API responses.
    pub public_base_url: String,
    /// Route of a category page on the Vue site; `{slug}` is replaced.
    pub menu_category_url: String,
    /// Route of a sub category page; `{category}` and `{slug}` are replaced.
    pub menu_sub_category_url: String,
    /// Route of an article page on the Vue site; `{slug}` is replaced.
    pub article_url: String,
    pub admin_seed: Option<AdminSeed>,
}

impl AppConfig {
    pub fn load() -> Self {
        dotenvy::dotenv().ok();

        let database_url = env::var("DATABASE_URL")
            .expect("DATABASE_URL must be specified in the .env file");

        let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET must be specified in the .env file");
        assert!(jwt_secret.len() >= 32, "JWT_SECRET must be at least 32 characters long");

        let server_port = env::var("SERVER_PORT").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
        let admin_bind = env::var("ADMIN_BIND").unwrap_or_else(|_| "127.0.0.1:4000".to_string());
        let frontend_origin =
            env::var("FRONTEND_ORIGIN").unwrap_or_else(|_| "http://localhost:5173".to_string());
        let cookie_secure = env::var("COOKIE_SECURE")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let upload_dir = env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".to_string());
        let public_base_url = env::var("PUBLIC_BASE_URL")
            .unwrap_or_else(|_| format!("http://{server_port}"))
            .trim_end_matches('/')
            .to_string();

        let menu_category_url = env::var("MENU_CATEGORY_URL").unwrap_or_else(|_| "/{slug}".to_string());
        let menu_sub_category_url = env::var("MENU_SUB_CATEGORY_URL").unwrap_or_else(|_| "/{category}/{slug}".to_string());

        // FIX: Replaced fallback pattern configuration to use clean slugs only
        let article_url = env::var("ARTICLE_URL").unwrap_or_else(|_| "/news/{slug}".to_string());

        let admin_seed = match (env::var("ADMIN_USERNAME"), env::var("ADMIN_PASSWORD")) {
            (Ok(username), Ok(password)) if !username.is_empty() && !password.is_empty() => {
                Some(AdminSeed {
                    username,
                    password,
                    name: env::var("ADMIN_NAME").unwrap_or_else(|_| "Administrator".into()),
                    email: env::var("ADMIN_EMAIL").unwrap_or_else(|_| "admin@example.com".into()),
                    phone: env::var("ADMIN_PHONE").unwrap_or_else(|_| "0000000000".into()),
                })
            }
            _ => None,
        };

        Self {
            database_url,
            server_port,
            admin_bind,
            jwt_secret,
            frontend_origin,
            cookie_secure,
            upload_dir,
            public_base_url,
            menu_category_url,
            menu_sub_category_url,
            article_url,
            admin_seed,
        }
    }
}
