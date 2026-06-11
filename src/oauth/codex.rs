use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    error::{AppError, AppResult},
    models::OAuthSession,
};

use super::{
    OAuthPkce, OAuthTokenGrant, decode_jwt_payload, expires_at_from_now, parse_jwt_expiration,
    parse_scope_string, summarize_oauth_error,
};

const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ORIGINATOR: &str = "codex_cli_rs";

/// Token-exchange grant + token types used to mint a Codex API key from the id_token.
/// Mirrors `obtain_api_key` in the upstream `codex` login crate.
const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const TOKEN_EXCHANGE_REQUESTED_TOKEN: &str = "openai-api-key";
const TOKEN_EXCHANGE_SUBJECT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";

const DEFAULT_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "api.connectors.read",
    "api.connectors.invoke",
];

/// Issuer used to build authorize/token/api-key URLs, with `KOU_CC_CODEX_ISSUER`
/// env override for tests. Mirrors `ServerOptions::issuer` (default
/// `https://auth.openai.com`) in the upstream `codex` login crate.
fn issuer() -> String {
    std::env::var("KOU_CC_CODEX_ISSUER").unwrap_or_else(|_| "https://auth.openai.com".to_string())
}

/// Authorization-code / api-key token endpoint, honoring the issuer override.
fn token_url() -> String {
    match std::env::var("KOU_CC_CODEX_ISSUER") {
        Ok(value) => format!("{}/oauth/token", value.trim_end_matches('/')),
        Err(_) => TOKEN_URL.to_string(),
    }
}

/// Refresh endpoint. Honors `CODEX_REFRESH_TOKEN_URL_OVERRIDE` (the upstream env
/// var) first, then the test issuer override, then the default token URL.
fn refresh_token_url() -> String {
    if let Ok(value) = std::env::var("CODEX_REFRESH_TOKEN_URL_OVERRIDE") {
        return value;
    }
    token_url()
}

/// Originator value, honoring the upstream `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` env var.
fn originator() -> String {
    std::env::var("CODEX_INTERNAL_ORIGINATOR_OVERRIDE").unwrap_or_else(|_| ORIGINATOR.to_string())
}

pub(super) fn default_scopes() -> Vec<String> {
    DEFAULT_SCOPES.iter().map(ToString::to_string).collect()
}

pub(super) fn build_authorization_url(
    redirect_uri: &str,
    state: &str,
    pkce: &OAuthPkce,
    scopes: &[String],
) -> String {
    let scope = if scopes.is_empty() {
        default_scopes().join(" ")
    } else {
        scopes.join(" ")
    };

    // Build the query string by hand, percent-encoding only the values, exactly
    // like `build_authorize_url` in the upstream `codex` login crate (which uses
    // `urlencoding::encode`). This is byte-for-byte compatible: spaces become
    // `%20` (not `+`) and reserved characters in `redirect_uri` are encoded.
    let issuer = issuer();
    let issuer = issuer.trim_end_matches('/');
    let originator = originator();
    let query: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect_uri),
        ("scope", &scope),
        ("code_challenge", &pkce.code_challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", state),
        ("originator", &originator),
    ];
    let qs = query
        .into_iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{issuer}/oauth/authorize?{qs}")
}

/// Percent-encodes a string the same way the `urlencoding` crate used by the
/// upstream `codex` login crate does: every byte except the RFC 3986 unreserved
/// set (`A-Za-z0-9` and `-`, `_`, `.`, `~`) is `%XX`-encoded with uppercase hex.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(hex_upper(byte >> 4));
                encoded.push(hex_upper(byte & 0x0F));
            }
        }
    }
    encoded
}

fn hex_upper(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

pub(super) async fn exchange_authorization_code(
    client: &Client,
    session: &OAuthSession,
    code: &str,
) -> AppResult<OAuthTokenGrant> {
    // Authorization-code exchange uses a form-encoded body, exactly like
    // `exchange_code_for_tokens` in the upstream `codex` login crate.
    let response = client
        .post(token_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", session.redirect_uri.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", session.code_verifier.as_str()),
        ])
        .send()
        .await?;

    let mut grant = parse_token_response(response).await?;

    // Token-exchange the id_token for a Codex API key (best effort), exactly
    // like `obtain_api_key` is called from the login callback in upstream codex.
    if let Some(id_token) = grant.id_token.as_deref() {
        grant.api_key = obtain_api_key(client, id_token).await.ok();
    }

    Ok(grant.into_token_grant())
}

pub(super) async fn refresh_access_token(
    client: &Client,
    refresh_token: &str,
) -> AppResult<OAuthTokenGrant> {
    // Refresh uses a JSON body plus the `originator` header, exactly like
    // `request_chatgpt_token_refresh` in the upstream `codex` login crate.
    let response = client
        .post(refresh_token_url())
        .header("Content-Type", "application/json")
        .header("originator", originator())
        .json(&serde_json::json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await?;
        // Mirror `classify_refresh_token_failure` from upstream codex: map known
        // 401 error codes to their canonical, user-facing messages.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::Upstream(classify_refresh_token_failure(&body)));
        }
        return Err(AppError::Upstream(format!(
            "codex oauth token request failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }

    let body = response.text().await?;
    let grant = parse_token_body(&body)?;
    // Note: per upstream `persist_tokens`, refresh does NOT re-mint the API key;
    // the stored key is preserved (handled via COALESCE in the repository).
    Ok(grant.into_token_grant())
}

/// Token-exchange the id_token for a long-lived Codex API key.
/// Faithful port of `obtain_api_key` in the upstream `codex` login crate.
async fn obtain_api_key(client: &Client, id_token: &str) -> AppResult<String> {
    #[derive(Deserialize)]
    struct ExchangeResp {
        access_token: String,
    }

    let response = client
        .post(token_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", TOKEN_EXCHANGE_GRANT_TYPE),
            ("client_id", CLIENT_ID),
            ("requested_token", TOKEN_EXCHANGE_REQUESTED_TOKEN),
            ("subject_token", id_token),
            ("subject_token_type", TOKEN_EXCHANGE_SUBJECT_TOKEN_TYPE),
        ])
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Upstream(format!(
            "codex api key exchange failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }

    let body: ExchangeResp = response.json().await?;
    Ok(body.access_token)
}

async fn parse_token_response(response: reqwest::Response) -> AppResult<CodexGrant> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "codex oauth token request failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }
    parse_token_body(&body)
}

fn parse_token_body(body: &str) -> AppResult<CodexGrant> {
    let token: CodexTokenResponse = serde_json::from_str(body)?;
    let id_token_claims = token
        .id_token
        .as_deref()
        .map(parse_id_token_claims)
        .transpose()?;

    let access_token = token.access_token.ok_or_else(|| {
        AppError::Upstream("codex oauth token response did not include an access token".into())
    })?;

    // expires_at: prefer explicit expires_in, then access-token JWT exp, then
    // id-token JWT exp. Upstream proactively refreshes off the access-token exp.
    let expires_at = expires_at_from_now(token.expires_in)
        .or_else(|| parse_jwt_expiration(&access_token).ok().flatten())
        .or_else(|| {
            token
                .id_token
                .as_deref()
                .and_then(|jwt| parse_jwt_expiration(jwt).ok().flatten())
        });

    let scopes = token
        .scope
        .as_deref()
        .and_then(|scope| parse_scope_string(Some(scope)))
        .or(token.scopes);

    // account_id strictly from the id_token's `chatgpt_account_id` claim, as in
    // upstream `persist_tokens_async`/`get_account_id`. This value populates the
    // `ChatGPT-Account-ID` upstream header.
    let remote_account_id = id_token_claims
        .as_ref()
        .and_then(|claims| claims.chatgpt_account_id.clone());
    let remote_email = id_token_claims
        .as_ref()
        .and_then(|claims| claims.email.clone());
    let is_fedramp = id_token_claims
        .as_ref()
        .map(|claims| claims.chatgpt_account_is_fedramp)
        .unwrap_or(false);

    Ok(CodexGrant {
        access_token,
        refresh_token: token.refresh_token,
        id_token: token.id_token,
        expires_at,
        scopes,
        remote_account_id,
        remote_email,
        is_fedramp,
        api_key: None,
    })
}

fn parse_id_token_claims(raw: &str) -> AppResult<CodexIdTokenClaims> {
    let claims: CodexIdTokenClaimsRaw = decode_jwt_payload(raw)?;
    let email = claims
        .email
        .or_else(|| claims.profile.and_then(|profile| profile.email));
    Ok(CodexIdTokenClaims {
        email,
        chatgpt_account_id: claims.auth.chatgpt_account_id,
        chatgpt_account_is_fedramp: claims.auth.chatgpt_account_is_fedramp,
    })
}

/// Maps a 401 refresh body to the upstream canonical message.
/// Faithful port of `classify_refresh_token_failure` + `extract_refresh_token_error_code`.
fn classify_refresh_token_failure(body: &str) -> String {
    const EXPIRED: &str = "Your access token could not be refreshed because your refresh token has expired. Please log out and sign in again.";
    const REUSED: &str = "Your access token could not be refreshed because your refresh token was already used. Please log out and sign in again.";
    const INVALIDATED: &str = "Your access token could not be refreshed because your refresh token was revoked. Please log out and sign in again.";
    const UNKNOWN: &str =
        "Your access token could not be refreshed. Please log out and sign in again.";

    let code = extract_refresh_token_error_code(body).map(|c| c.to_ascii_lowercase());
    match code.as_deref() {
        Some("refresh_token_expired") => EXPIRED.to_string(),
        Some("refresh_token_reused") => REUSED.to_string(),
        Some("refresh_token_invalidated") => INVALIDATED.to_string(),
        _ => UNKNOWN.to_string(),
    }
}

fn extract_refresh_token_error_code(body: &str) -> Option<String> {
    if body.trim().is_empty() {
        return None;
    }
    let map = match serde_json::from_str::<Value>(body).ok()? {
        Value::Object(map) => map,
        _ => return None,
    };
    if let Some(error_value) = map.get("error") {
        match error_value {
            Value::Object(obj) => {
                if let Some(code) = obj.get("code").and_then(Value::as_str) {
                    return Some(code.to_string());
                }
            }
            Value::String(code) => return Some(code.to_string()),
            _ => {}
        }
    }
    map.get("code").and_then(Value::as_str).map(str::to_string)
}

/// Internal, fully-parsed grant including the raw id_token (needed for the
/// API-key token exchange) before lowering to the shared `OAuthTokenGrant`.
struct CodexGrant {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    scopes: Option<Vec<String>>,
    remote_account_id: Option<String>,
    remote_email: Option<String>,
    is_fedramp: bool,
    api_key: Option<String>,
}

impl CodexGrant {
    fn into_token_grant(self) -> OAuthTokenGrant {
        OAuthTokenGrant {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at: self.expires_at,
            scopes: self.scopes,
            remote_account_id: self.remote_account_id,
            remote_email: self.remote_email,
            is_fedramp: self.is_fedramp,
            api_key: self.api_key,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CodexTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

struct CodexIdTokenClaims {
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    chatgpt_account_is_fedramp: bool,
}

#[derive(Debug, Deserialize)]
struct CodexIdTokenClaimsRaw {
    #[serde(default)]
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    profile: Option<CodexProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: CodexAuthClaims,
}

#[derive(Debug, Deserialize)]
struct CodexProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CodexAuthClaims {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    chatgpt_account_is_fedramp: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    fn make_jwt(claims: &serde_json::Value) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(b"{\"alg\":\"none\",\"typ\":\"JWT\"}");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(claims).unwrap());
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"sig");
        format!("{header}.{payload}.{sig}")
    }

    #[test]
    fn authorize_url_matches_codex_query_shape() {
        let pkce = OAuthPkce {
            code_verifier: "verifier".into(),
            code_challenge: "challenge".into(),
        };
        let url = build_authorization_url(
            "http://localhost:1455/auth/callback",
            "state-xyz",
            &pkce,
            &[],
        );
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("code_challenge=challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("id_token_add_organizations=true"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("originator=codex_cli_rs"));
        // Codex percent-encodes spaces as %20 (urlencoding crate), not `+`.
        assert!(url.contains("scope=openid%20profile%20email%20offline_access"));
        assert!(!url.contains('+'));
        // redirect_uri reserved chars are percent-encoded.
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
    }

    #[test]
    fn account_id_comes_from_chatgpt_account_id_claim_only() {
        let id_token = make_jwt(&json!({
            "email": "u@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-123",
                "chatgpt_user_id": "user-should-not-be-used",
                "chatgpt_account_is_fedramp": true,
            }
        }));
        let access = make_jwt(&json!({"exp": 4102444800_i64}));
        let body = json!({
            "access_token": access,
            "refresh_token": "rt",
            "id_token": id_token,
            "scope": "openid profile",
        })
        .to_string();

        let grant = parse_token_body(&body).unwrap();
        assert_eq!(grant.remote_account_id.as_deref(), Some("acc-123"));
        assert_eq!(grant.remote_email.as_deref(), Some("u@example.com"));
        assert!(grant.is_fedramp);
    }

    #[test]
    fn missing_auth_claim_yields_no_account_and_no_fedramp() {
        let id_token = make_jwt(&json!({ "email": "u@example.com" }));
        let access = make_jwt(&json!({"exp": 4102444800_i64}));
        let body = json!({
            "access_token": access,
            "id_token": id_token,
        })
        .to_string();
        let grant = parse_token_body(&body).unwrap();
        assert_eq!(grant.remote_account_id, None);
        assert!(!grant.is_fedramp);
        assert_eq!(grant.remote_email.as_deref(), Some("u@example.com"));
    }

    #[test]
    fn email_falls_back_to_profile_claim() {
        let id_token = make_jwt(&json!({
            "https://api.openai.com/profile": { "email": "profile@example.com" },
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-1" }
        }));
        let body = json!({
            "access_token": make_jwt(&json!({})),
            "id_token": id_token,
        })
        .to_string();
        let grant = parse_token_body(&body).unwrap();
        assert_eq!(grant.remote_email.as_deref(), Some("profile@example.com"));
    }

    #[test]
    fn refresh_error_codes_map_to_canonical_messages() {
        assert!(
            classify_refresh_token_failure(r#"{"error":{"code":"refresh_token_expired"}}"#)
                .contains("expired")
        );
        assert!(
            classify_refresh_token_failure(r#"{"error":"refresh_token_reused"}"#)
                .contains("already used")
        );
        assert!(
            classify_refresh_token_failure(r#"{"code":"refresh_token_invalidated"}"#)
                .contains("revoked")
        );
        assert!(
            classify_refresh_token_failure(r#"{"error":"something_else"}"#)
                .contains("could not be refreshed")
        );
    }

    #[test]
    fn extract_error_code_handles_shapes() {
        assert_eq!(
            extract_refresh_token_error_code(r#"{"error":{"code":"x"}}"#).as_deref(),
            Some("x")
        );
        assert_eq!(
            extract_refresh_token_error_code(r#"{"error":"y"}"#).as_deref(),
            Some("y")
        );
        assert_eq!(
            extract_refresh_token_error_code(r#"{"code":"z"}"#).as_deref(),
            Some("z")
        );
        assert_eq!(extract_refresh_token_error_code(""), None);
        assert_eq!(extract_refresh_token_error_code("plain text"), None);
    }
}
