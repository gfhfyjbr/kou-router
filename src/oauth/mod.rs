mod claude;
mod codex;
pub mod service;

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

pub use service::{
    OAuthCompleteAuthorizationRequest, OAuthService, OAuthStartAuthorizationRequest,
    OAuthStartAuthorizationResponse,
};

#[derive(Debug, Clone)]
pub(crate) struct OAuthPkce {
    pub(crate) code_verifier: String,
    pub(crate) code_challenge: String,
}

#[derive(Debug, Clone)]
pub(crate) struct OAuthTokenGrant {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) scopes: Option<Vec<String>>,
    pub(crate) remote_account_id: Option<String>,
    pub(crate) remote_email: Option<String>,
}

pub(crate) fn generate_pkce() -> OAuthPkce {
    let mut bytes = [0_u8; 64];
    rand::rng().fill_bytes(&mut bytes);
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    OAuthPkce {
        code_verifier,
        code_challenge,
    }
}

pub(crate) fn generate_state() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn expires_at_from_now(expires_in_seconds: Option<i64>) -> Option<DateTime<Utc>> {
    expires_in_seconds.map(|seconds| Utc::now() + Duration::seconds(seconds.max(0)))
}

pub(crate) fn parse_scope_string(scope: Option<&str>) -> Option<Vec<String>> {
    scope.map(|value| {
        value
            .split_whitespace()
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    })
}

pub(crate) fn decode_jwt_payload<T: DeserializeOwned>(jwt: &str) -> AppResult<T> {
    let mut parts = jwt.split('.');
    let (_, payload_b64, _) = match (parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(signature))
            if !header.is_empty() && !payload.is_empty() && !signature.is_empty() =>
        {
            (header, payload, signature)
        }
        _ => {
            return Err(AppError::Upstream(
                "oauth response contained an invalid ID token".into(),
            ));
        }
    };

    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| AppError::Upstream("oauth response contained an invalid ID token".into()))?;
    serde_json::from_slice(&payload)
        .map_err(|_| AppError::Upstream("oauth response contained an invalid ID token".into()))
}

pub(crate) fn parse_jwt_expiration(jwt: &str) -> AppResult<Option<DateTime<Utc>>> {
    let payload: Value = decode_jwt_payload(jwt)?;
    Ok(payload
        .get("exp")
        .and_then(Value::as_i64)
        .and_then(|value| DateTime::<Utc>::from_timestamp(value, 0)))
}

pub(crate) fn summarize_oauth_error(status: reqwest::StatusCode, body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|json| {
            json.get("error_description")
                .and_then(Value::as_str)
                .or_else(|| {
                    json.get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(Value::as_str)
                })
                .or_else(|| json.get("message").and_then(Value::as_str))
                .or_else(|| json.get("error").and_then(Value::as_str))
        })
        .unwrap_or("request failed");
    format!("status {status}: {message}")
}
