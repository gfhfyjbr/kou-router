use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    routing::delete,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::{
    auth::{
        self, ApiKeyCreated, AuthStatus, CreateApiKeyRequest, LoginRequest, ManagementAuth,
        SetupRequest,
    },
    audio::AudioService,
    error::{AppError, AppResult},
    models::{
        CreateAliasRequest, EndpointKind, HealthResponse, NewCombo, NewProviderConnection,
        SettingsPayload,
    },
    presets::{import_request_to_provider, provider_presets, ImportProviderPresetRequest},
    repository::SqliteRepository,
    service::{RoutedResult, RouterService},
    upstream::PassthroughHeaders,
};

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<SqliteRepository>,
    pub service: RouterService,
    pub audio: AudioService,
}

impl AppState {
    pub fn new(repository: Arc<SqliteRepository>) -> Self {
        let service = RouterService::new(repository.clone());
        let audio = AudioService::new(repository.clone());
        Self {
            repository,
            service,
            audio,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1", get(list_models))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/completions", post(completions))
        .route("/v1/messages", post(messages))
        .route("/v1/responses", post(responses))
        .route("/v1/responses/{*path}", post(responses_with_suffix))
        .route("/v1/api/chat", post(ollama_chat))
        .route("/v1/embeddings", get(list_embedding_models).post(embeddings))
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
        .route("/api/combos", get(list_combos).post(create_combo))
        .route("/api/models/alias", get(list_aliases).post(upsert_alias))
        .route("/api/settings", get(get_settings).post(put_settings))
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
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::ChatCompletions, payload, None, None).await
}

async fn completions(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::Completions, payload, None, None).await
}

async fn messages(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    let pt = PassthroughHeaders::from_header_map(&headers);
    route_json(state, EndpointKind::Messages, payload, None, Some(pt)).await
}

async fn responses(State(state): State<AppState>, Json(payload): Json<Value>) -> AppResult<Response> {
    route_json(state, EndpointKind::Responses, payload, None, None).await
}

async fn responses_with_suffix(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::Responses, payload, Some(path), None).await
}

async fn ollama_chat(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::OllamaChat, payload, None, None).await
}

async fn embeddings(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::Embeddings, payload, None, None).await
}

async fn image_generations(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
 ) -> AppResult<Response> {
    route_json(state, EndpointKind::ImagesGenerations, payload, None, None).await
}

async fn music_generations(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
 ) -> AppResult<Response> {
    route_json(state, EndpointKind::MusicGenerations, payload, None, None).await
}

async fn video_generations(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
 ) -> AppResult<Response> {
    route_json(state, EndpointKind::VideosGenerations, payload, None, None).await
}

async fn moderations(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::Moderations, payload, None, None).await
}

async fn rerank(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::Rerank, payload, None, None).await
}

async fn search(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> AppResult<Response> {
    route_json(state, EndpointKind::Search, payload, None, None).await
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
    let routed = state.service.route(endpoint, payload, suffix, passthrough_headers).await?;
    match routed {
        RoutedResult::Json(r) => {
            let mut body = r.body;
            if let Some(map) = body.as_object_mut() {
                map.insert("_kou_router".to_string(), serde_json::to_value(r.debug)?);
            }
            if r.is_stream {
                Ok((
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    body["raw"].as_str().unwrap_or_default().to_string(),
                )
                    .into_response())
            } else {
                Ok(Json(body).into_response())
            }
        }
        RoutedResult::Stream(s) => {
            let body = Body::from_stream(s.stream);
            let debug_json = serde_json::to_string(&s.debug).unwrap_or_default();
            let mut builder = Response::builder()
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .header("connection", "keep-alive");
            if let Ok(val) = axum::http::HeaderValue::from_str(&debug_json) {
                builder = builder.header("x-kou-debug", val);
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
    Ok(Json(state.repository.create_provider_connection(create).await?))
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

async fn put_settings(
    State(state): State<AppState>,
    Json(payload): Json<SettingsPayload>,
) -> AppResult<Json<Value>> {
    Ok(Json(state.service.put_settings(payload).await?))
}


// ── Auth routes ────────────────────────────────────────────────────

async fn auth_status(State(state): State<AppState>) -> AppResult<Json<AuthStatus>> {
    let auth_required = state.repository.get_setting_bool("require_auth").await.unwrap_or(false);
    let setup_complete = state.repository.get_setting_string("admin_password_hash").await.is_ok();
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
    if state.repository.get_setting_string("admin_password_hash").await.is_ok() {
        return Err(AppError::BadRequest("admin password already configured".into()));
    }
    if payload.password.len() < 8 {
        return Err(AppError::BadRequest("password must be at least 8 characters".into()));
    }
    let hash = auth::password::hash_password(&payload.password)?;
    state.repository.set_setting("admin_password_hash", &format!("\"{hash}\"")).await?;

    // Generate JWT secret if not present
    if state.repository.get_setting_string("jwt_secret").await.is_err() {
        let secret = auth::jwt::generate_jwt_secret();
        state.repository.set_setting("jwt_secret", &format!("\"{secret}\"")).await?;
    }

    // Enable auth
    state.repository.set_setting("require_auth", "true").await?;

    Ok(Json(json!({"status": "ok", "message": "Admin password set and auth enabled"})))
}

async fn auth_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<Response> {
    let hash = state.repository.get_setting_string("admin_password_hash").await
        .map_err(|_| AppError::BadRequest("auth not configured, run setup first".into()))?;
    if !auth::password::verify_password(&payload.password, &hash)? {
        return Err(AppError::BadRequest("unauthorized: invalid password".into()));
    }
    let secret = state.repository.get_setting_string("jwt_secret").await
        .map_err(|_| AppError::BadRequest("JWT secret not configured".into()))?;
    let token = auth::jwt::create_token(&secret, "admin")?;

    let cookie = format!("kou_auth={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400");
    Ok((
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(json!({"status": "ok", "message": "logged in"})),
    ).into_response())
}

async fn auth_logout() -> Response {
    let cookie = "kou_auth=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0";
    (
        [(axum::http::header::SET_COOKIE, cookie.to_string())],
        Json(json!({"status": "ok", "message": "logged out"})),
    ).into_response()
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
    state.repository.create_api_key(
        &created.id,
        &created.name,
        &key_hash,
        &created.key_prefix,
        &payload.allowed_models,
    ).await?;
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