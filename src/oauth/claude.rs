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
        .post(TOKEN_URL)
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
        .post(TOKEN_URL)
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
    Ok(OAuthTokenGrant {
        access_token: token.access_token.ok_or_else(|| {
            AppError::Upstream("claude oauth token response did not include an access token".into())
        })?,
        refresh_token: token.refresh_token,
        expires_at: expires_at_from_now(token.expires_in),
        scopes: token
            .scope
            .as_deref()
            .and_then(|scope| parse_scope_string(Some(scope)))
            .or(token.scopes),
        remote_account_id: None,
        remote_email: None,
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
}
