use std::time::Duration;

use reqwest::{Client, Url};
use serde::Deserialize;

use crate::{
    error::{AppError, AppResult},
    models::OAuthSession,
};

use super::{
    OAuthPkce, OAuthTokenGrant, expires_at_from_now, parse_scope_string, summarize_oauth_error,
};

const AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const API_KEY_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/create_api_key";
const ROLES_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/roles";

/// OAuth endpoints, with env overrides for tests.
fn authorize_url() -> String {
    std::env::var("KOU_CC_CLAUDE_AUTHORIZE_URL").unwrap_or_else(|_| AUTHORIZE_URL.to_string())
}

fn token_url() -> String {
    std::env::var("KOU_CC_CLAUDE_TOKEN_URL").unwrap_or_else(|_| TOKEN_URL.to_string())
}

fn profile_url() -> String {
    std::env::var("KOU_CC_CLAUDE_PROFILE_URL").unwrap_or_else(|_| PROFILE_URL.to_string())
}

fn api_key_url() -> String {
    std::env::var("KOU_CC_CLAUDE_API_KEY_URL").unwrap_or_else(|_| API_KEY_URL.to_string())
}

fn roles_url() -> String {
    std::env::var("KOU_CC_CLAUDE_ROLES_URL").unwrap_or_else(|_| ROLES_URL.to_string())
}

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const INFERENCE_SCOPE: &str = "user:inference";
const DEFAULT_SCOPES: &[&str] = &[
    "org:create_api_key",
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];
const REFRESH_SCOPES: &[&str] = &[
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
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
    let mut url = Url::parse(&authorize_url())
        .map_err(|_| AppError::Upstream("invalid claude oauth authorize url".into()))?;
    url.query_pairs_mut()
        .append_pair("code", "true")
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
            "grant_type": "authorization_code",
            "code": code,
            "state": session.state,
            "client_id": CLIENT_ID,
            "redirect_uri": session.redirect_uri,
            "code_verifier": session.code_verifier,
        }))
        .send()
        .await?;

    let mut grant = parse_token_response(response).await?;
    if grant.scopes.is_none() {
        grant.scopes = Some(session.scopes.clone());
    }
    enrich_with_profile(client, &mut grant).await?;
    let _ = fetch_user_roles(client, &grant.access_token).await;
    if !has_inference_scope(grant.scopes.as_deref()) {
        grant.api_key = Some(create_api_key(client, &grant.access_token).await?);
    }
    Ok(grant)
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
            "scope": refresh_scope(),
        }))
        .send()
        .await?;

    parse_token_response(response).await
}

fn refresh_scope() -> String {
    REFRESH_SCOPES.join(" ")
}

fn has_inference_scope(scopes: Option<&[String]>) -> bool {
    scopes
        .map(|scopes| scopes.iter().any(|scope| scope == INFERENCE_SCOPE))
        .unwrap_or(false)
}

async fn enrich_with_profile(client: &Client, grant: &mut OAuthTokenGrant) -> AppResult<()> {
    let profile = fetch_profile(client, &grant.access_token).await?;
    if let Some(account) = profile.account {
        if grant.remote_account_id.is_none() {
            grant.remote_account_id = account.uuid;
        }
        if grant.remote_email.is_none() {
            grant.remote_email = account.email.or(account.email_address);
        }
    }
    Ok(())
}

async fn fetch_profile(client: &Client, access_token: &str) -> AppResult<ClaudeProfileResponse> {
    let response = client
        .get(profile_url())
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "claude oauth profile request failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }
    Ok(serde_json::from_str(&body)?)
}

async fn fetch_user_roles(client: &Client, access_token: &str) -> AppResult<()> {
    let response = client
        .get(roles_url())
        .bearer_auth(access_token)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "claude oauth roles request failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }
    Ok(())
}

async fn create_api_key(client: &Client, access_token: &str) -> AppResult<String> {
    let response = client
        .post(api_key_url())
        .bearer_auth(access_token)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "claude oauth api key request failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }
    let created: ClaudeApiKeyResponse = serde_json::from_str(&body)?;
    created.raw_key.ok_or_else(|| {
        AppError::Upstream("claude oauth api key response did not include raw_key".into())
    })
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
        is_fedramp: false,
        api_key: None,
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

#[derive(Debug, Deserialize)]
struct ClaudeProfileResponse {
    #[serde(default)]
    account: Option<ClaudeProfileAccount>,
}

#[derive(Debug, Deserialize)]
struct ClaudeProfileAccount {
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeApiKeyResponse {
    #[serde(default)]
    raw_key: Option<String>,
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
    fn test_default_scopes_match_claude_cli_order() {
        assert_eq!(
            default_scopes(),
            vec![
                "org:create_api_key",
                "user:profile",
                "user:inference",
                "user:sessions:claude_code",
                "user:mcp_servers",
                "user:file_upload",
            ]
        );
    }

    #[test]
    fn test_build_authorization_url_matches_claude_cli() {
        let pkce = OAuthPkce {
            code_verifier: "verifier".to_string(),
            code_challenge: "challenge".to_string(),
        };
        let url = build_authorization_url(
            "https://platform.claude.com/oauth/code/callback",
            "state",
            &pkce,
            &default_scopes(),
        )
        .unwrap();

        assert_eq!(
            url,
            "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=org%3Acreate_api_key+user%3Aprofile+user%3Ainference+user%3Asessions%3Aclaude_code+user%3Amcp_servers+user%3Afile_upload&code_challenge=challenge&code_challenge_method=S256&state=state"
        );
    }

    #[test]
    fn test_refresh_scope_matches_claude_cli_default() {
        assert_eq!(
            refresh_scope(),
            "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"
        );
    }

    #[test]
    fn test_has_inference_scope_controls_api_key_path() {
        assert!(has_inference_scope(Some(&["user:inference".to_string()])));
        assert!(!has_inference_scope(Some(&["user:profile".to_string()])));
        assert!(!has_inference_scope(None));
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
            token
                .account
                .as_ref()
                .and_then(|a| a.email_address.clone())
                .unwrap(),
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
