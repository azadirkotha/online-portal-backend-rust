use axum::{extract::State, response::Response, Extension};
use tower_sessions::Session;

use crate::admin::helpers::{build_layout, internal_error, render};
use crate::admin::middleware::CurrentAdmin;
use crate::admin::views::DashboardTemplate;
use crate::state::AppState;

pub async fn index(
    State(state): State<AppState>,
    Extension(admin): Extension<CurrentAdmin>,
    session: Session,
) -> Response {
    let pool = &state.db_pool;
    let counts = async {
        let users = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users").fetch_one(pool).await?;
        let roles = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM roles").fetch_one(pool).await?;
        let perms = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM permissions").fetch_one(pool).await?;
        Ok::<_, sqlx::Error>((users, roles, perms))
    }
    .await;

    match counts {
        Ok((users, roles, permissions)) => {
            let layout = build_layout(&session, &admin).await;
            render(DashboardTemplate { layout, users, roles, permissions })
        }
        Err(e) => internal_error(e),
    }
}
