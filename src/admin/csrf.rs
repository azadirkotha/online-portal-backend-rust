use rand::{distributions::Alphanumeric, Rng};
use tower_sessions::Session;

const KEY: &str = "csrf";

/// Returns the session's CSRF token, creating one if needed.
pub async fn token(session: &Session) -> String {
    if let Ok(Some(t)) = session.get::<String>(KEY).await {
        return t;
    }
    let t: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();
    let _ = session.insert(KEY, t.clone()).await;
    t
}

/// Constant-time comparison of the submitted token against the session's token.
pub async fn verify(session: &Session, submitted: &str) -> bool {
    match session.get::<String>(KEY).await {
        Ok(Some(expected)) => ct_eq(expected.as_bytes(), submitted.as_bytes()),
        _ => false,
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
