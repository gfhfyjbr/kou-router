use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use uuid::Uuid;

use super::models::Claims;
use crate::error::{AppError, AppResult};

const JWT_EXPIRY_HOURS: i64 = 24;

pub fn create_token(secret: &str, username: &str) -> AppResult<String> {
    let now = chrono::Utc::now();
    let claims = Claims {
        sub: username.to_string(),
        exp: (now + chrono::Duration::hours(JWT_EXPIRY_HOURS)).timestamp() as usize,
        iat: now.timestamp() as usize,
        jti: Uuid::new_v4().to_string(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::BadRequest(format!("failed to create token: {e}")))
}

pub fn verify_token(secret: &str, token: &str) -> AppResult<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|e| AppError::BadRequest(format!("invalid token: {e}")))
}

/// Generate a random JWT secret
pub fn generate_jwt_secret() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
