use std::sync::Arc;

use axum::{
    Json, Router,
    body::Body,
    extract::{Multipart, Path, Query, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    audio::AudioService,
    auth::{
        self, ApiKeyCreated, AuthStatus, CreateApiKeyRequest, LoginRequest, ManagementAuth,
        SetupRequest,
    },
    error::{AppError, AppResult},
    models::{
        CreateAliasRequest, EndpointKind, HealthResponse, NewCombo, NewProviderAccount,
        NewProviderConnection, OAuthSession, ProviderAccount, ProviderAccountAuthMode,
        SettingsPayload,
    },
    oauth::{OAuthCompleteAuthorizationRequest, OAuthService, OAuthStartAuthorizationRequest},
    presets::{ImportProviderPresetRequest, import_request_to_provider, provider_presets},
    repository::SqliteRepository,
    service::{RoutedResult, RouterService},
    upstream::PassthroughHeaders,
};

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<SqliteRepository>,
    pub service: RouterService,
    pub audio: AudioService,
    pub oauth: OAuthService,
}

impl AppState {
    pub fn new(repository: Arc<SqliteRepository>) -> Self {
        let service = RouterService::new(repository.clone());
        let audio = AudioService::new(repository.clone());
        let oauth = OAuthService::new(repository.clone());
        Self {
            repository,
            service,
            audio,
            oauth,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ListProviderAccountsQuery {
    provider_connection_id: String,
}

#[derive(Debug, Deserialize)]
struct ProviderAccountOAuthStartBody {
    provider_connection_id: String,
    #[serde(default)]
    provider_account_id: Option<String>,
    redirect_uri: String,
}

#[derive(Debug, Deserialize)]
struct ProviderAccountOAuthCallbackBody {
    state: String,
    code: String,
}

#[derive(Debug, Serialize)]
struct ProviderAccountResponse {
    id: String,
    provider_connection_id: String,
    label: Option<String>,
    auth_mode: ProviderAccountAuthMode,
    has_api_key: bool,
    has_access_token: bool,
    has_refresh_token: bool,
    expires_at: Option<DateTime<Utc>>,
    scopes: Vec<String>,
    remote_account_id: Option<String>,
    remote_email: Option<String>,
    enabled: bool,
    priority: i64,
    last_used_at: Option<DateTime<Utc>>,
    rate_limited_until: Option<DateTime<Utc>>,
    circuit_open_until: Option<DateTime<Utc>>,
    last_error: Option<String>,
    last_error_type: Option<String>,
    last_refresh_at: Option<DateTime<Utc>>,
    refresh_error: Option<String>,
    backoff_level: i64,
    consecutive_use_count: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ProviderAccount> for ProviderAccountResponse {
    fn from(account: ProviderAccount) -> Self {
        Self {
            id: account.id,
            provider_connection_id: account.provider_connection_id,
            label: account.label,
            auth_mode: account.auth_mode,
            has_api_key: has_secret(account.api_key.as_deref()),
            has_access_token: has_secret(account.access_token.as_deref()),
            has_refresh_token: has_secret(account.refresh_token.as_deref()),
            expires_at: account.expires_at,
            scopes: account.scopes,
            remote_account_id: account.remote_account_id,
            remote_email: account.remote_email,
            enabled: account.enabled,
            priority: account.priority,
            last_used_at: account.last_used_at,
            rate_limited_until: account.rate_limited_until,
            circuit_open_until: account.circuit_open_until,
            last_error: account.last_error,
            last_error_type: account.last_error_type,
            last_refresh_at: account.last_refresh_at,
            refresh_error: account.refresh_error,
            backoff_level: account.backoff_level,
            consecutive_use_count: account.consecutive_use_count,
            created_at: account.created_at,
            updated_at: account.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct OAuthSessionResponse {
    id: String,
    state: String,
    provider_connection_id: String,
    provider_account_id: Option<String>,
    redirect_uri: String,
    scopes: Vec<String>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

impl From<OAuthSession> for OAuthSessionResponse {
    fn from(session: OAuthSession) -> Self {
        Self {
            id: session.id,
            state: session.state,
            provider_connection_id: session.provider_connection_id,
            provider_account_id: session.provider_account_id,
            redirect_uri: session.redirect_uri,
            scopes: session.scopes,
            created_at: session.created_at,
            expires_at: session.expires_at,
            consumed_at: session.consumed_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct ProviderAccountOAuthStartResponse {
    session: OAuthSessionResponse,
    authorization_url: String,
}

fn has_secret(value: Option<&str>) -> bool {
    value.is_some_and(|secret| !secret.trim().is_empty())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1", get(list_models))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/completions", post(completions))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(messages_count_tokens))
        .route("/v1/responses", post(responses))
        .route("/v1/responses/{*path}", post(responses_with_suffix))
        .route("/v1/api/chat", post(ollama_chat))
        .route(
            "/v1/embeddings",
            get(list_embedding_models).post(embeddings),
        )
        .route(
            "/v1/images/generations",
            get(list_image_models).post(image_generations),
        )
        .route(
            "/v1/music/generations",
            get(list_music_models).post(music_generations),
        )
        .route(
            "/v1/videos/generations",
            get(list_video_models).post(video_generations),
        )
        .route("/v1/moderations", post(moderations))
        .route("/v1/rerank", post(rerank))
        .route("/v1/search", get(list_search_models).post(search))
        .route("/v1/audio/speech", post(audio_speech))
        .route("/v1/audio/transcriptions", post(audio_transcriptions))
        .route("/api/providers", get(list_providers).post(create_provider))
        .route("/api/providers/presets", get(list_provider_presets))
        .route("/api/providers/import", post(import_provider_preset))
        .route(
            "/api/provider-accounts",
            get(list_provider_accounts).post(create_provider_account),
        )
        .route(
            "/api/provider-accounts/oauth/start",
            post(start_provider_account_oauth),
        )
        .route(
            "/api/provider-accounts/oauth/callback",
            post(complete_provider_account_oauth),
        )
        .route(
            "/api/provider-accounts/{id}/refresh",
            post(refresh_provider_account),
        )
        .route(
            "/api/provider-accounts/{id}/disable",
            post(disable_provider_account),
        )
        .route(
            "/api/provider-accounts/{id}/enable",
            post(enable_provider_account),
        )
        .route(
            "/api/provider-accounts/{id}",
            delete(delete_provider_account),
        )
        .route("/api/combos", get(list_combos).post(create_combo))
        .route("/api/models/alias", get(list_aliases).post(upsert_alias))
        .route("/api/settings", get(get_settings).post(put_settings))
        .route("/api/ratelimits", get(get_ratelimits))
        // Auth routes (public)
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/setup", post(auth_setup))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        // API key management (requires ManagementAuth)
        .route("/api/keys", get(list_api_keys).post(create_api_key))
        .route("/api/keys/{id}", delete(revoke_api_key))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "kou-router",
    })
}

async fn list_models(
    State(state): State<AppState>,
) -> AppResult<Json<crate::models::OpenAiModelsResponse>> {
    Ok(Json(state.service.list_models().await?))
}

async fn list_embedding_models(
    State(state): State<AppState>,
) -> AppResult<Json<crate::models::OpenAiModelsResponse>> {
    Ok(Json(
        state
            .service
            .list_models_for_endpoint(EndpointKind::Embeddings)
            .await?,
    ))
}

async fn list_image_models(
    State(state): State<AppState>,
) -> AppResult<Json<crate::models::OpenAiModelsResponse>> {
    Ok(Json(
        state
            .service
            .list_models_for_endpoint(EndpointKind::ImagesGenerations)
            .await?,
    ))
}

async fn list_music_models(
    State(state): State<AppState>,
) -> AppResult<Json<crate::models::OpenAiModelsResponse>> {
    Ok(Json(
        state
            .service
            .list_models_for_endpoint(EndpointKind::MusicGenerations)
            .await?,
    ))
}

async fn list_video_models(
    State(state): State<AppState>,
) -> AppResult<Json<crate::models::OpenAiModelsResponse>> {
    Ok(Json(
        state
            .service
            .list_models_for_endpoint(EndpointKind::VideosGenerations)
            .await?,
    ))
}

async fn list_search_models(
    State(state): State<AppState>,
) -> AppResult<Json<crate::models::OpenAiModelsResponse>> {
    Ok(Json(
        state
            .service
            .list_models_for_endpoint(EndpointKind::Search)
            .await?,
    ))
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(
        state,
        EndpointKind::ChatCompletions,
        payload,
        None,
        Some(pt),
    )
    .await
}

async fn completions(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Completions, payload, None, Some(pt)).await
}

async fn messages(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Messages, payload, None, Some(pt)).await
}

/// Proxy endpoint for /v1/messages/count_tokens — passes request directly
/// to the upstream Anthropic provider without translation.
async fn messages_count_tokens(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    let request_id = headers
        .get("x-request-id")
        .or_else(|| headers.get("x-client-request-id"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let (status, resp_headers, body) = state
        .service
        .proxy_raw_to_provider(request_id.clone(), "/messages/count_tokens", &payload, Some(pt))
        .await?;

    let mut builder = Response::builder().status(status);
    // Forward upstream response headers
    for (name, value) in &resp_headers {
        if let (Ok(hn), Ok(hv)) = (
            axum::http::header::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(value),
        ) {
            builder = builder.header(hn, hv);
        }
    }
    builder = builder.header(axum::http::header::CONTENT_TYPE, "application/json");
    Ok(builder.body(Body::from(body)).unwrap().into_response())
}

async fn responses(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Responses, payload, None, Some(pt)).await
}

async fn responses_with_suffix(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(path): Path<String>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(
        state,
        EndpointKind::Responses,
        payload,
        Some(path),
        Some(pt),
    )
    .await
}

async fn ollama_chat(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::OllamaChat, payload, None, Some(pt)).await
}

async fn embeddings(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Embeddings, payload, None, Some(pt)).await
}

async fn image_generations(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(
        state,
        EndpointKind::ImagesGenerations,
        payload,
        None,
        Some(pt),
    )
    .await
}

async fn music_generations(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(
        state,
        EndpointKind::MusicGenerations,
        payload,
        None,
        Some(pt),
    )
    .await
}

async fn video_generations(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(
        state,
        EndpointKind::VideosGenerations,
        payload,
        None,
        Some(pt),
    )
    .await
}

async fn moderations(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Moderations, payload, None, Some(pt)).await
}

async fn rerank(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Rerank, payload, None, Some(pt)).await
}

async fn search(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Search, payload, None, Some(pt)).await
}

async fn audio_speech(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    state.audio.speech(&state.service, payload).await
}

async fn audio_transcriptions(
    State(state): State<AppState>,
    multipart: Multipart,
) -> AppResult<Response> {
    state.audio.transcriptions(&state.service, multipart).await
}

async fn route_json(
    state: AppState,
    endpoint: EndpointKind,
    payload: Value,
    suffix: Option<String>,
    passthrough_headers: Option<PassthroughHeaders>,
) -> AppResult<Response> {
    // Generate or reuse client-provided request ID
    let request_id = passthrough_headers
        .as_ref()
        .and_then(|pt| {
            pt.headers.iter().find_map(|(name, value)| {
                let n = name.to_ascii_lowercase();
                if n == "x-request-id" || n == "x-client-request-id" {
                    Some(value.clone())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let routed = state
        .service
        .route(
            endpoint,
            payload,
            suffix,
            passthrough_headers,
            request_id.clone(),
        )
        .await?;
    match routed {
        RoutedResult::Json(r) => {
            let mut body = r.body;
            let debug_json = serde_json::to_string(&r.debug).unwrap_or_default();
            let mut builder = Response::builder().header("x-request-id", &request_id);
            if r.is_stream {
                if let Ok(val) = axum::http::HeaderValue::from_str(&debug_json) {
                    builder = builder.header("x-kou-debug", val);
                }
            } else if let Some(map) = body.as_object_mut() {
                map.insert("_kou_router".to_string(), serde_json::to_value(&r.debug)?);
            }
            // Attach upstream response headers (rate-limit, retry-after, request-id)
            for (name, value) in &r.response_headers {
                if let (Ok(hn), Ok(hv)) = (
                    axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                    axum::http::HeaderValue::from_str(value),
                ) {
                    builder = builder.header(hn, hv);
                }
            }
            if r.is_stream {
                Ok(builder
                    .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(
                        body["raw"].as_str().unwrap_or_default().to_string(),
                    ))
                    .unwrap()
                    .into_response())
            } else {
                let json_bytes = serde_json::to_vec(&body)?;
                Ok(builder
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json_bytes))
                    .unwrap()
                    .into_response())
            }
        }
        RoutedResult::Stream(s) => {
            let body = Body::from_stream(s.stream);
            let mut builder = Response::builder()
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .header("connection", "keep-alive")
                .header("x-request-id", &request_id);
            // Attach upstream response headers (rate-limit, retry-after, request-id)
            for (name, value) in &s.response_headers {
                if let (Ok(hn), Ok(hv)) = (
                    axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                    axum::http::HeaderValue::from_str(value),
                ) {
                    builder = builder.header(hn, hv);
                }
            }
            Ok(builder.body(body).unwrap().into_response())
        }
    }
}

async fn list_providers(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<crate::models::ProviderConnection>>> {
    Ok(Json(state.repository.list_provider_connections().await?))
}

async fn create_provider(
    State(state): State<AppState>,
    Json(payload): Json<NewProviderConnection>,
) -> AppResult<Json<crate::models::ProviderConnection>> {
    Ok(Json(
        state.repository.create_provider_connection(payload).await?,
    ))
}

async fn list_provider_presets() -> AppResult<Json<Vec<crate::presets::ProviderPreset>>> {
    Ok(Json(provider_presets()))
}

async fn import_provider_preset(
    State(state): State<AppState>,
    Json(payload): Json<ImportProviderPresetRequest>,
) -> AppResult<Json<crate::models::ProviderConnection>> {
    let create = import_request_to_provider(payload)?;
    Ok(Json(
        state.repository.create_provider_connection(create).await?,
    ))
}

async fn list_combos(State(state): State<AppState>) -> AppResult<Json<Vec<crate::models::Combo>>> {
    Ok(Json(state.repository.list_combos().await?))
}

async fn create_combo(
    State(state): State<AppState>,
    Json(payload): Json<NewCombo>,
) -> AppResult<Json<crate::models::Combo>> {
    Ok(Json(state.repository.create_combo(payload).await?))
}

async fn list_aliases(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<crate::models::ModelAlias>>> {
    Ok(Json(state.repository.list_aliases().await?))
}

async fn upsert_alias(
    State(state): State<AppState>,
    Json(payload): Json<CreateAliasRequest>,
) -> AppResult<Json<Value>> {
    let alias = state
        .repository
        .upsert_alias(&payload.alias, &payload.target)
        .await?;
    Ok(Json(json!(alias)))
}

async fn get_settings(State(state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(state.service.get_settings().await?))
}

async fn get_ratelimits(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let states = state.service.rate_limit_tracker.get_all_states();
    Ok(Json(json!({ "ratelimits": states })))
}

async fn put_settings(
    State(state): State<AppState>,
    Json(payload): Json<SettingsPayload>,
) -> AppResult<Json<Value>> {
    Ok(Json(state.service.put_settings(payload).await?))
}

async fn list_provider_accounts(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Query(query): Query<ListProviderAccountsQuery>,
) -> AppResult<Json<Vec<ProviderAccountResponse>>> {
    state
        .repository
        .find_provider_connection_by_id(&query.provider_connection_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "provider connection {}",
                query.provider_connection_id
            ))
        })?;

    let accounts = state
        .repository
        .list_provider_accounts(&query.provider_connection_id)
        .await?;
    Ok(Json(
        accounts
            .into_iter()
            .map(ProviderAccountResponse::from)
            .collect(),
    ))
}

async fn create_provider_account(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Json(payload): Json<NewProviderAccount>,
) -> AppResult<Json<ProviderAccountResponse>> {
    let account = state.repository.create_provider_account(payload).await?;
    Ok(Json(account.into()))
}

async fn start_provider_account_oauth(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Json(payload): Json<ProviderAccountOAuthStartBody>,
) -> AppResult<Json<ProviderAccountOAuthStartResponse>> {
    if let Some(provider_account_id) = payload.provider_account_id.as_deref() {
        let provider_account = state
            .repository
            .get_provider_account(provider_account_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("provider account {provider_account_id}")))?;

        if provider_account.provider_connection_id != payload.provider_connection_id {
            return Err(AppError::BadRequest(format!(
                "provider account '{provider_account_id}' does not belong to provider connection '{}'",
                payload.provider_connection_id
            )));
        }
    }

    let response = state
        .oauth
        .start_authorization(OAuthStartAuthorizationRequest {
            provider_connection_id: payload.provider_connection_id,
            provider_account_id: payload.provider_account_id,
            redirect_uri: payload.redirect_uri,
        })
        .await?;

    Ok(Json(ProviderAccountOAuthStartResponse {
        session: response.session.into(),
        authorization_url: response.authorization_url,
    }))
}

async fn complete_provider_account_oauth(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Json(payload): Json<ProviderAccountOAuthCallbackBody>,
) -> AppResult<Json<ProviderAccountResponse>> {
    let account = state
        .oauth
        .complete_authorization(OAuthCompleteAuthorizationRequest {
            state: payload.state,
            code: payload.code,
        })
        .await?;
    Ok(Json(account.into()))
}

async fn refresh_provider_account(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Path(id): Path<String>,
) -> AppResult<Json<ProviderAccountResponse>> {
    let account = state.oauth.refresh_provider_account(&id).await?;
    Ok(Json(account.into()))
}

async fn disable_provider_account(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let updated = state
        .repository
        .set_provider_account_enabled(&id, false)
        .await?;
    if updated {
        Ok(Json(json!({"status": "ok", "id": id, "enabled": false})))
    } else {
        Err(AppError::NotFound(format!("provider account {id}")))
    }
}

async fn enable_provider_account(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let updated = state
        .repository
        .set_provider_account_enabled(&id, true)
        .await?;
    if updated {
        Ok(Json(json!({"status": "ok", "id": id, "enabled": true})))
    } else {
        Err(AppError::NotFound(format!("provider account {id}")))
    }
}

async fn delete_provider_account(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let deleted = state.repository.delete_provider_account(&id).await?;
    if deleted {
        Ok(Json(json!({"status": "ok", "id": id})))
    } else {
        Err(AppError::NotFound(format!("provider account {id}")))
    }
}

// ── Auth routes ────────────────────────────────────────────────────

async fn auth_status(State(state): State<AppState>) -> AppResult<Json<AuthStatus>> {
    let auth_required = state
        .repository
        .get_setting_bool("require_auth")
        .await
        .unwrap_or(false);
    let setup_complete = state
        .repository
        .get_setting_string("admin_password_hash")
        .await
        .is_ok();
    Ok(Json(AuthStatus {
        auth_required,
        setup_complete,
    }))
}

async fn auth_setup(
    State(state): State<AppState>,
    Json(payload): Json<SetupRequest>,
) -> AppResult<Json<Value>> {
    // Don't allow re-setup if already configured
    if state
        .repository
        .get_setting_string("admin_password_hash")
        .await
        .is_ok()
    {
        return Err(AppError::BadRequest(
            "admin password already configured".into(),
        ));
    }
    if payload.password.len() < 8 {
        return Err(AppError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }
    let hash = auth::password::hash_password(&payload.password)?;
    state
        .repository
        .set_setting("admin_password_hash", &format!("\"{hash}\""))
        .await?;

    // Generate JWT secret if not present
    if state
        .repository
        .get_setting_string("jwt_secret")
        .await
        .is_err()
    {
        let secret = auth::jwt::generate_jwt_secret();
        state
            .repository
            .set_setting("jwt_secret", &format!("\"{secret}\""))
            .await?;
    }

    // Enable auth
    state.repository.set_setting("require_auth", "true").await?;

    Ok(Json(
        json!({"status": "ok", "message": "Admin password set and auth enabled"}),
    ))
}

async fn auth_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<Response> {
    let hash = state
        .repository
        .get_setting_string("admin_password_hash")
        .await
        .map_err(|_| AppError::BadRequest("auth not configured, run setup first".into()))?;
    if !auth::password::verify_password(&payload.password, &hash)? {
        return Err(AppError::BadRequest(
            "unauthorized: invalid password".into(),
        ));
    }
    let secret = state
        .repository
        .get_setting_string("jwt_secret")
        .await
        .map_err(|_| AppError::BadRequest("JWT secret not configured".into()))?;
    let token = auth::jwt::create_token(&secret, "admin")?;

    let cookie = format!("kou_auth={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400");
    Ok((
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(json!({"status": "ok", "message": "logged in"})),
    )
        .into_response())
}

async fn auth_logout() -> Response {
    let cookie = "kou_auth=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0";
    (
        [(axum::http::header::SET_COOKIE, cookie.to_string())],
        Json(json!({"status": "ok", "message": "logged out"})),
    )
        .into_response()
}

async fn list_api_keys(
    State(state): State<AppState>,
    _auth: ManagementAuth,
) -> AppResult<Json<Vec<auth::ApiKeyRecord>>> {
    Ok(Json(state.repository.list_api_keys().await?))
}

async fn create_api_key(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Json(payload): Json<CreateApiKeyRequest>,
) -> AppResult<Json<ApiKeyCreated>> {
    let created = auth::api_key::generate_api_key(&payload.name);
    let key_hash = auth::api_key::hash_api_key(&created.key);
    state
        .repository
        .create_api_key(
            &created.id,
            &created.name,
            &key_hash,
            &created.key_prefix,
            &payload.allowed_models,
        )
        .await?;
    Ok(Json(created))
}

async fn revoke_api_key(
    State(state): State<AppState>,
    _auth: ManagementAuth,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let deleted = state.repository.revoke_api_key(&id).await?;
    if deleted {
        Ok(Json(json!({"status": "ok", "id": id})))
    } else {
        Err(AppError::NotFound(format!("API key {id}")))
    }
}
