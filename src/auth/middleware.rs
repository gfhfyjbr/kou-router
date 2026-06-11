use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};

use crate::{
    error::{AppError, AppResult},
    routes::AppState,
};

use super::{api_key::hash_api_key, jwt::verify_token, models::AuthContext};

/// Extractor for proxy routes. Validates `Authorization: Bearer <api_key>`
/// or `x-api-key: <key>` header (Claude Code compatibility).
/// Returns `AuthContext::Anonymous` when auth is not required.
pub struct ProxyAuth(pub AuthContext);

/// Extractor for management routes. Tries JWT cookie `kou_auth` first,
/// then falls back to `Authorization: Bearer <api_key>`.
/// Returns `AuthContext::Anonymous` when auth is not required.
pub struct ManagementAuth(pub AuthContext);

/// Best-effort identity for proxy routes: resolves an API key when one is
/// presented but never rejects the request. Proxy endpoints stay open by
/// design (see `test_proxy_blocked_without_key`); this only attributes
/// request-log entries to a key.
pub struct ProxyIdentity(pub AuthContext);

fn unauthorized(msg: &str) -> AppError {
    AppError::BadRequest(format!("unauthorized: {msg}"))
}

pub async fn is_auth_required(state: &AppState) -> bool {
    state
        .repository
        .get_setting_bool("require_auth")
        .await
        .unwrap_or(false)
}

async fn get_jwt_secret(state: &AppState) -> Option<String> {
    state.repository.get_setting_string("jwt_secret").await.ok()
}

fn extract_bearer_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

fn extract_api_key_header(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get("x-api-key")?
        .to_str()
        .ok()
        .map(String::from)
}

fn extract_jwt_cookie(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get("cookie")?.to_str().ok()?;
    for cookie in cookies.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix("kou_auth=") {
            return Some(value.to_string());
        }
    }
    None
}

/// True when the request carries a valid admin JWT cookie.
/// Used by the web UI gate: without it the SPA itself is not served.
pub async fn has_valid_admin_cookie(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = extract_jwt_cookie(headers) else {
        return false;
    };
    let Some(secret) = get_jwt_secret(state).await else {
        return false;
    };
    verify_token(&secret, &token).is_ok()
}

async fn validate_api_key(state: &AppState, key: &str) -> AppResult<AuthContext> {
    let key_hash = hash_api_key(key);
    match state.repository.find_api_key_by_hash(&key_hash).await? {
        Some(record) if record.is_active => {
            state.repository.touch_api_key_usage(&record.id).await?;
            Ok(AuthContext::ApiKey {
                key_id: record.id,
                key_name: record.name,
                allowed_models: record.allowed_models,
            })
        }
        Some(_) => Err(unauthorized("API key is disabled")),
        None => Err(unauthorized("invalid API key")),
    }
}

impl FromRequestParts<AppState> for ProxyAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !is_auth_required(state).await {
            return Ok(Self(AuthContext::Anonymous));
        }

        let token = extract_bearer_token(parts)
            .or_else(|| extract_api_key_header(parts))
            .ok_or_else(|| unauthorized("missing Authorization or x-api-key header"))?;

        let ctx = validate_api_key(state, &token).await?;
        Ok(Self(ctx))
    }
}

impl FromRequestParts<AppState> for ProxyIdentity {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(token) = extract_bearer_token(parts).or_else(|| extract_api_key_header(parts))
            && let Ok(ctx) = validate_api_key(state, &token).await
        {
            return Ok(Self(ctx));
        }
        Ok(Self(AuthContext::Anonymous))
    }
}

impl FromRequestParts<AppState> for ManagementAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !is_auth_required(state).await {
            return Ok(Self(AuthContext::Anonymous));
        }

        // Try JWT cookie first.
        if let Some(token) = extract_jwt_cookie(&parts.headers) {
            if let Some(secret) = get_jwt_secret(state).await {
                if let Ok(claims) = verify_token(&secret, &token) {
                    return Ok(Self(AuthContext::Admin {
                        username: claims.sub,
                    }));
                }
            }
            // Cookie present but invalid — fall through to bearer token.
        }

        // Fall back to bearer API key or x-api-key header.
        if let Some(token) = extract_bearer_token(parts).or_else(|| extract_api_key_header(parts)) {
            let ctx = validate_api_key(state, &token).await?;
            return Ok(Self(ctx));
        }

        Err(unauthorized("no valid credentials provided"))
    }
}
