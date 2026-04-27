use std::sync::Arc;

use chrono::{Duration, Utc};
use serde::Serialize;

use crate::{
    error::{AppError, AppResult},
    models::{
        NewOAuthSession, NewProviderAccount, OAuthSession, ProviderAccount, ProviderAccountAuthMode,
    },
    proxy::{self, ProxyClientCache},
    repository::SqliteRepository,
};

use super::{OAuthTokenGrant, claude, codex, generate_pkce, generate_state};

#[derive(Clone)]
pub struct OAuthService {
    repository: Arc<SqliteRepository>,
    clients: ProxyClientCache,
}

#[derive(Debug, Clone)]
pub struct OAuthStartAuthorizationRequest {
    pub provider_connection_id: String,
    pub provider_account_id: Option<String>,
    pub redirect_uri: String,
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthCompleteAuthorizationRequest {
    pub state: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthStartAuthorizationResponse {
    pub session: OAuthSession,
    pub authorization_url: String,
}

impl OAuthService {
    pub fn new(repository: Arc<SqliteRepository>) -> Self {
        Self::with_clients(repository, ProxyClientCache::new())
    }

    pub fn with_clients(repository: Arc<SqliteRepository>, clients: ProxyClientCache) -> Self {
        Self {
            repository,
            clients,
        }
    }

    pub async fn start_authorization(
        &self,
        request: OAuthStartAuthorizationRequest,
    ) -> AppResult<OAuthStartAuthorizationResponse> {
        self.repository.delete_expired_oauth_sessions().await?;
        let provider_connection = self
            .repository
            .find_provider_connection_by_id(&request.provider_connection_id)
            .await?
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "provider connection '{}' does not exist",
                    request.provider_connection_id
                ))
            })?;

        let provider = OAuthProvider::from_provider_id(&provider_connection.provider)?;
        let proxy_url = normalize_proxy_url(request.proxy_url.as_deref())?;
        if let Some(provider_account_id) = request.provider_account_id.as_deref() {
            self.repository
                .update_provider_account_proxy(provider_account_id, proxy_url.as_deref())
                .await?;
        }
        let pkce = generate_pkce();
        let state = generate_state();
        let scopes = provider.default_scopes();
        let authorization_url =
            provider.build_authorization_url(&request.redirect_uri, &state, &pkce, &scopes)?;
        let session = self
            .repository
            .create_oauth_session(NewOAuthSession {
                state,
                provider_connection_id: provider_connection.id,
                provider_account_id: request.provider_account_id,
                redirect_uri: request.redirect_uri,
                code_verifier: pkce.code_verifier,
                scopes: Some(scopes),
                proxy_url,
                expires_at: Utc::now() + Duration::minutes(10),
            })
            .await?;

        Ok(OAuthStartAuthorizationResponse {
            session,
            authorization_url,
        })
    }

    pub async fn complete_authorization(
        &self,
        request: OAuthCompleteAuthorizationRequest,
    ) -> AppResult<ProviderAccount> {
        let session = self
            .repository
            .get_oauth_session_by_state(&request.state)
            .await?
            .ok_or_else(|| AppError::BadRequest("oauth session not found".into()))?;

        if session.consumed_at.is_some() {
            return Err(AppError::BadRequest(
                "oauth session has already been consumed".into(),
            ));
        }
        if session.expires_at <= Utc::now() {
            self.repository.delete_oauth_session(&session.id).await?;
            return Err(AppError::BadRequest("oauth session has expired".into()));
        }

        let provider_connection = self
            .repository
            .find_provider_connection_by_id(&session.provider_connection_id)
            .await?
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "provider connection '{}' does not exist",
                    session.provider_connection_id
                ))
            })?;
        let provider = OAuthProvider::from_provider_id(&provider_connection.provider)?;
        let client = self.clients.for_optional(session.proxy_url.as_deref())?;
        let token_grant = provider
            .exchange_authorization_code(&client, &session, &request.code)
            .await?;
        let scopes = token_grant
            .scopes
            .clone()
            .unwrap_or_else(|| session.scopes.clone());

        if !self
            .repository
            .mark_oauth_session_consumed(&session.id)
            .await?
        {
            return Err(AppError::BadRequest(
                "oauth session has already been consumed".into(),
            ));
        }

        let account = if let Some(account_id) = &session.provider_account_id {
            let updated = self
                .repository
                .update_provider_account_oauth_credentials(
                    account_id,
                    &token_grant.access_token,
                    token_grant.refresh_token.as_deref(),
                    token_grant.expires_at,
                    &scopes,
                    token_grant.remote_account_id.as_deref(),
                    token_grant.remote_email.as_deref(),
                )
                .await?;
            if !updated {
                return Err(AppError::NotFound(format!("provider account {account_id}")));
            }
            self.repository
                .update_provider_account_proxy(account_id, session.proxy_url.as_deref())
                .await?;
            self.repository
                .get_provider_account(account_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("provider account {account_id}")))?
        } else {
            self.repository
                .create_provider_account(NewProviderAccount {
                    provider_connection_id: session.provider_connection_id.clone(),
                    label: token_grant
                        .remote_email
                        .clone()
                        .or_else(|| token_grant.remote_account_id.clone()),
                    auth_mode: ProviderAccountAuthMode::OAuth,
                    api_key: None,
                    access_token: Some(token_grant.access_token.clone()),
                    refresh_token: token_grant.refresh_token.clone(),
                    expires_at: token_grant.expires_at,
                    scopes: Some(scopes.clone()),
                    remote_account_id: token_grant.remote_account_id.clone(),
                    remote_email: token_grant.remote_email.clone(),
                    enabled: true,
                    priority: None,
                    proxy_url: session.proxy_url.clone(),
                })
                .await?
        };
        self.repository.delete_oauth_session(&session.id).await?;
        Ok(account)
    }

    pub async fn refresh_provider_account(
        &self,
        provider_account_id: &str,
    ) -> AppResult<ProviderAccount> {
        let account = self
            .repository
            .get_provider_account(provider_account_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("provider account {provider_account_id}")))?;
        if account.auth_mode != ProviderAccountAuthMode::OAuth {
            return Err(AppError::BadRequest(format!(
                "provider account '{provider_account_id}' does not use oauth"
            )));
        }
        let refresh_token = account.refresh_token.clone().ok_or_else(|| {
            AppError::BadRequest(format!(
                "provider account '{provider_account_id}' has no refresh token"
            ))
        })?;

        let provider_connection = self
            .repository
            .find_provider_connection_by_id(&account.provider_connection_id)
            .await?
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "provider connection '{}' does not exist",
                    account.provider_connection_id
                ))
            })?;
        let provider = OAuthProvider::from_provider_id(&provider_connection.provider)?;
        let client = self.clients.for_optional(account.proxy_url.as_deref())?;

        let token_grant = match provider.refresh_access_token(&client, &refresh_token).await {
            Ok(grant) => grant,
            Err(err) => {
                self.repository
                    .record_provider_account_refresh_error(provider_account_id, &err.to_string())
                    .await?;
                return Err(err);
            }
        };

        let scopes = token_grant
            .scopes
            .clone()
            .unwrap_or_else(|| account.scopes.clone());
        let remote_account_id = token_grant
            .remote_account_id
            .as_deref()
            .or(account.remote_account_id.as_deref());
        let remote_email = token_grant
            .remote_email
            .as_deref()
            .or(account.remote_email.as_deref());

        let updated = self
            .repository
            .update_provider_account_oauth_credentials(
                provider_account_id,
                &token_grant.access_token,
                token_grant.refresh_token.as_deref(),
                token_grant.expires_at,
                &scopes,
                remote_account_id,
                remote_email,
            )
            .await?;
        if !updated {
            return Err(AppError::NotFound(format!(
                "provider account {provider_account_id}"
            )));
        }

        self.repository
            .get_provider_account(provider_account_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("provider account {provider_account_id}")))
    }
}

fn normalize_proxy_url(proxy_url: Option<&str>) -> AppResult<Option<String>> {
    match proxy_url.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            proxy::parse_and_validate(value)?;
            Ok(Some(value.to_owned()))
        }
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Copy)]
enum OAuthProvider {
    Codex,
    Claude,
}

impl OAuthProvider {
    fn from_provider_id(provider_id: &str) -> AppResult<Self> {
        match provider_id.to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude-oauth" | "claude" | "anthropic-oauth" => Ok(Self::Claude),
            other => Err(AppError::BadRequest(format!(
                "provider '{other}' does not support oauth"
            ))),
        }
    }

    fn default_scopes(self) -> Vec<String> {
        match self {
            Self::Codex => codex::default_scopes(),
            Self::Claude => claude::default_scopes(),
        }
    }

    fn build_authorization_url(
        self,
        redirect_uri: &str,
        state: &str,
        pkce: &super::OAuthPkce,
        scopes: &[String],
    ) -> AppResult<String> {
        match self {
            Self::Codex => Ok(codex::build_authorization_url(
                redirect_uri,
                state,
                pkce,
                scopes,
            )),
            Self::Claude => claude::build_authorization_url(redirect_uri, state, pkce, scopes),
        }
    }

    async fn exchange_authorization_code(
        self,
        client: &reqwest::Client,
        session: &OAuthSession,
        code: &str,
    ) -> AppResult<OAuthTokenGrant> {
        match self {
            Self::Codex => codex::exchange_authorization_code(client, session, code).await,
            Self::Claude => claude::exchange_authorization_code(client, session, code).await,
        }
    }

    async fn refresh_access_token(
        self,
        client: &reqwest::Client,
        refresh_token: &str,
    ) -> AppResult<OAuthTokenGrant> {
        match self {
            Self::Codex => codex::refresh_access_token(client, refresh_token).await,
            Self::Claude => claude::refresh_access_token(client, refresh_token).await,
        }
    }
}
