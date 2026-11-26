use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use super::helpers::internal_error;
use crate::models::permissions::effective_for_user;
use crate::state::AppState;

/// Inserted into request extensions by `require_admin`; handlers read it with `Extension<CurrentAdmin>`.
#[derive(Clone)]
pub struct CurrentAdmin {
    pub id: i32,
    pub username: String,
    /// full name and photo, for the sidebar
    pub name: String,
    pub photo: String,
    pub permissions: Vec<String>,
}

impl CurrentAdmin {
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }
}

/// Session guard. Permissions are re-read from the database on EVERY request,
/// so removing a role or deactivating a user takes effect immediately.
pub async fn require_admin(
    State(state): State<AppState>,
    session: Session,
    mut req: Request,
    next: Next,
) -> Response {
    let user_id: Option<i32> = session.get("user_id").await.ok().flatten();
    let Some(user_id) = user_id else {
        return Redirect::to("/login").into_response();
    };

    let row: Result<Option<(String, String, String)>, sqlx::Error> =
        sqlx::query_as("SELECT username, name, photo FROM users WHERE id = ? AND status = 1")
            .bind(user_id)
            .fetch_optional(&state.db_pool)
            .await;

    let (username, name, photo) = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            let _ = session.flush().await;
            return Redirect::to("/login").into_response();
        }
        Err(e) => return internal_error(e),
    };

    let permissions = match effective_for_user(&state.db_pool, user_id).await {
        Ok(p) => p,
        Err(e) => return internal_error(e),
    };

    if !permissions.iter().any(|p| p == "admin:access") {
        let _ = session.flush().await;
        return Redirect::to("/login").into_response();
    }

    req.extensions_mut().insert(CurrentAdmin { id: user_id, username, name, photo, permissions });
    next.run(req).await
}
