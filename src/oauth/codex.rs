use reqwest::{Client, Url};
use serde::Deserialize;

use crate::{
    error::{AppError, AppResult},
    models::OAuthSession,
};

use super::{
    OAuthPkce, OAuthTokenGrant, decode_jwt_payload, expires_at_from_now, parse_jwt_expiration,
    parse_scope_string, summarize_oauth_error,
};

const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ORIGINATOR: &str = "codex_cli_rs";
const DEFAULT_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "api.connectors.read",
    "api.connectors.invoke",
];

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

    let mut url = Url::parse(AUTHORIZE_URL).expect("codex oauth authorize url should be valid");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", &scope)
        .append_pair("code_challenge", &pkce.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", ORIGINATOR);

    url.into()
}

pub(super) async fn exchange_authorization_code(
    client: &Client,
    session: &OAuthSession,
    code: &str,
) -> AppResult<OAuthTokenGrant> {
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", session.redirect_uri.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", session.code_verifier.as_str()),
        ])
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
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
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
            "codex oauth token request failed: {}",
            summarize_oauth_error(status, &body)
        )));
    }

    let token: CodexTokenResponse = serde_json::from_str(&body)?;
    let id_token_claims = token
        .id_token
        .as_deref()
        .map(parse_id_token_claims)
        .transpose()?;

    let expires_at = expires_at_from_now(token.expires_in)
        .or_else(|| {
            token
                .access_token
                .as_deref()
                .and_then(|jwt| parse_jwt_expiration(jwt).ok().flatten())
        })
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

    Ok(OAuthTokenGrant {
        access_token: token.access_token.ok_or_else(|| {
            AppError::Upstream("codex oauth token response did not include an access token".into())
        })?,
        refresh_token: token.refresh_token,
        expires_at,
        scopes,
        remote_account_id: id_token_claims
            .as_ref()
            .and_then(|claims| claims.auth.chatgpt_account_id.clone())
            .or_else(|| {
                id_token_claims
                    .as_ref()
                    .and_then(|claims| claims.auth.chatgpt_user_id())
            })
            .or(token.account_id),
        remote_email: id_token_claims.and_then(|claims| {
            claims
                .email
                .or(claims.profile.and_then(|profile| profile.email))
        }),
    })
}

fn parse_id_token_claims(raw: &str) -> AppResult<CodexIdTokenClaims> {
    decode_jwt_payload(raw)
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
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexIdTokenClaims {
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
    chatgpt_user_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
}

impl CodexAuthClaims {
    fn chatgpt_user_id(&self) -> Option<String> {
        self.chatgpt_user_id
            .clone()
            .or_else(|| self.user_id.clone())
    }
}
