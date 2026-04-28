use reqwest::{Client, Url};
use serde::Deserialize;

use crate::{
    error::{AppError, AppResult},
    models::OAuthSession,
};

use super::{
    OAuthPkce, OAuthTokenGrant, expires_at_from_now, parse_scope_string, summarize_oauth_error,
};

const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

/// Token endpoint, with `KOU_CC_CLAUDE_TOKEN_URL` env override for tests.
fn token_url() -> String {
    std::env::var("KOU_CC_CLAUDE_TOKEN_URL").unwrap_or_else(|_| TOKEN_URL.to_string())
}
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const DEFAULT_SCOPES: &[&str] = &[
    "user:file_upload",
    "user:inference",
    "user:mcp_servers",
    "user:profile",
    "user:sessions:claude_code",
];

pub(super) fn default_scopes() -> Vec<String> {
    DEFAULT_SCOPES.iter().map(ToString::to_string).collect()
}

pub(super) fn build_authorization_url(
    redirect_uri: &str,
    state: &str,
    pkce: &OAuthPkce,
    scopes: &[String],
) -> AppResult<String> {
    let scope = if scopes.is_empty() {
        default_scopes().join(" ")
    } else {
        scopes.join(" ")
    };
    let mut url = Url::parse(AUTHORIZE_URL)
        .map_err(|_| AppError::Upstream("invalid claude oauth authorize url".into()))?;
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", &scope)
        .append_pair("code_challenge", &pkce.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    Ok(url.into())
}

pub(super) async fn exchange_authorization_code(
    client: &Client,
    session: &OAuthSession,
    code: &str,
) -> AppResult<OAuthTokenGrant> {
    let code = code.split_once('#').map_or(code, |(value, _)| value);
    let response = client
        .post(token_url())
        .json(&serde_json::json!({
            "code": code,
            "state": session.state,
            "grant_type": "authorization_code",
            "client_id": CLIENT_ID,
            "redirect_uri": session.redirect_uri,
            "code_verifier": session.code_verifier,
        }))
        .send()
        .await?;

    parse_token_response(response).await
}

pub(super) async fn refresh_access_token(
    client: &Client,
    refresh_token: &str,
) -> AppResult<OAuthTokenGrant> {
    let response = client
        .post(token_url())
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
        }))
        .send()
        .await?;

    parse_token_response(response).await
}

async fn parse_token_response(response: reqwest::Response) -> AppResult<OAuthTokenGrant> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "claude oauth token request failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }

    let token: ClaudeTokenResponse = serde_json::from_str(&body)?;
    let access_token = token.access_token.ok_or_else(|| {
        AppError::Upstream("claude oauth token response did not include an access token".into())
    })?;

    let account_email = token.account.as_ref().and_then(|a| a.email_address.clone());
    let account_uuid = token
        .account
        .as_ref()
        .and_then(|a| a.uuid.clone())
        .or_else(|| extract_jwt_sub(&access_token));

    Ok(OAuthTokenGrant {
        access_token,
        refresh_token: token.refresh_token,
        expires_at: expires_at_from_now(token.expires_in),
        scopes: token
            .scope
            .as_deref()
            .and_then(|scope| parse_scope_string(Some(scope)))
            .or(token.scopes),
        remote_account_id: account_uuid,
        remote_email: account_email,
    })
}

#[derive(Debug, Deserialize)]
struct ClaudeTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    #[serde(default)]
    account: Option<ClaudeAccountClaims>,
    #[serde(default)]
    #[allow(dead_code)]
    organization: Option<ClaudeOrgClaims>,
}

#[derive(Debug, Deserialize)]
struct ClaudeAccountClaims {
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    email_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOrgClaims {
    #[serde(default)]
    #[allow(dead_code)]
    uuid: Option<String>,
}

/// Fallback: попытаться декодировать access_token как JWT и достать `sub` claim.
/// Anthropic OAuth tokens — JWT с `sub` = account uuid.
fn extract_jwt_sub(jwt: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct SubClaim {
        #[serde(default)]
        sub: Option<String>,
    }
    let claims: SubClaim = super::decode_jwt_payload(jwt).ok()?;
    claims.sub.filter(|s| !s.is_empty())
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
    fn test_parse_token_response_extracts_account_uuid() {
        let body = json!({
            "access_token": "sk-ant-xxx",
            "refresh_token": "refr-xxx",
            "expires_in": 3600,
            "scope": "user:profile user:inference",
            "account": {
                "uuid": "acc-uuid-from-response",
                "email_address": "user@example.com"
            },
            "organization": { "uuid": "org-uuid-x" }
        });
        let token: ClaudeTokenResponse = serde_json::from_value(body).unwrap();
        assert_eq!(
            token.account.as_ref().and_then(|a| a.uuid.clone()).unwrap(),
            "acc-uuid-from-response"
        );
        assert_eq!(
            token.account.as_ref().and_then(|a| a.email_address.clone()).unwrap(),
            "user@example.com"
        );
    }

    #[test]
    fn test_extract_jwt_sub_basic() {
        let jwt = make_jwt(&json!({"sub": "acc-uuid-from-jwt"}));
        assert_eq!(extract_jwt_sub(&jwt), Some("acc-uuid-from-jwt".to_string()));
    }

    #[test]
    fn test_extract_jwt_sub_missing_returns_none() {
        let jwt = make_jwt(&json!({"exp": 1700000000}));
        assert_eq!(extract_jwt_sub(&jwt), None);
    }

    #[test]
    fn test_extract_jwt_sub_empty_string_returns_none() {
        let jwt = make_jwt(&json!({"sub": ""}));
        assert_eq!(extract_jwt_sub(&jwt), None);
    }

    #[test]
    fn test_extract_jwt_sub_invalid_jwt_returns_none() {
        assert_eq!(extract_jwt_sub("not-a-jwt"), None);
        assert_eq!(extract_jwt_sub(""), None);
        assert_eq!(extract_jwt_sub("a.b"), None);
    }
}
