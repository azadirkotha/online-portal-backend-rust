use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct RegisterInput {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct AuthInput {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub token: String,
}

/// JWT claims for the public API.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Claims {
    pub sub: i32,                 // User ID
    pub permissions: Vec<String>, // Effective permissions (direct + via roles)
    pub exp: usize,               // Expiration timestamp
}
