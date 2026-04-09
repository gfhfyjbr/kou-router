use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Represents a verified identity on a request.
#[derive(Debug, Clone)]
pub enum AuthContext {
    ApiKey {
        key_id: String,
        key_name: String,
        allowed_models: Vec<String>,
    },
    Admin {
        username: String,
    },
    Anonymous,
}

/// API key as stored in DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub allowed_models: Vec<String>,
    pub is_active: bool,
    pub last_used_at: Option<DateTime<Utc>>,
    pub usage_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Response when creating a new API key.
#[derive(Debug, Serialize)]
pub struct ApiKeyCreated {
    pub id: String,
    pub name: String,
    pub key: String,
    pub key_prefix: String,
}

/// JWT claims.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
    pub jti: String,
}

/// Login request.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

/// Setup request (set initial admin password).
#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    pub password: String,
}

/// Auth status response.
#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub auth_required: bool,
    pub setup_complete: bool,
}

/// Create API key request.
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default = "default_allowed_models")]
    pub allowed_models: Vec<String>,
}

fn default_allowed_models() -> Vec<String> {
    vec!["*".to_string()]
}
