use std::sync::Arc;

use axum::{
    extract::Multipart,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;

use crate::{
    error::{AppError, AppResult},
    models::{AudioTranscriptionPayload, EndpointKind, RoutingDebug},
    repository::SqliteRepository,
    service::RouterService,
    upstream::{fallback_error, AudioResponse, UpstreamClient},
};

#[derive(Clone)]
pub struct AudioService {
    repository: Arc<SqliteRepository>,
    upstream: UpstreamClient,
}

impl AudioService {
    pub fn new(repository: Arc<SqliteRepository>) -> Self {
        Self {
            repository,
            upstream: UpstreamClient::new(),
        }
    }

    pub async fn speech(&self, _router: &RouterService, payload: Value) -> AppResult<Response> {
        let normalized = RouterService::new(self.repository.clone()).normalize_audio_speech(payload)?;
        let resolved_model = self.repository.resolve_alias(&normalized.model).await?;
        let providers = self.eligible_providers(EndpointKind::AudioSpeech).await?;
        let (candidate, provider) = resolve_provider(&providers, &resolved_model, EndpointKind::AudioSpeech)?;
        let (_, raw_model) = split_model(&candidate)?;

        let upstream = self
            .upstream
            .execute_audio_speech(&provider, &raw_model, &normalized.body)
            .await?;
        let debug = RoutingDebug {
            requested_model: normalized.model,
            resolved_model,
            endpoint: EndpointKind::AudioSpeech.as_str().to_string(),
            path_suffix: None,
            tried: vec![upstream.as_attempt(provider.id.clone(), candidate)],
        };

        if (200..300).contains(&upstream.status.as_u16()) {
            self.repository.mark_provider_success(&provider.id).await?;
            return build_audio_response(upstream, debug);
        }

        self.repository
            .mark_provider_failure(&provider.id, &upstream.body_preview())
            .await?;
        Err(AppError::Upstream(format!(
            "audio speech request failed for {}",
            provider.provider
        )))
    }

    pub async fn transcriptions(
        &self,
        _router: &RouterService,
        multipart: Multipart,
    ) -> AppResult<Response> {
        let normalized = collect_transcription_payload(multipart).await?;
        let requested_model = normalized.model.clone();
        let resolved_model = self.repository.resolve_alias(&requested_model).await?;
        let providers = self
            .eligible_providers(EndpointKind::AudioTranscriptions)
            .await?;
        let (candidate, provider) =
            resolve_provider(&providers, &resolved_model, EndpointKind::AudioTranscriptions)?;
        let (_, raw_model) = split_model(&candidate)?;

        let upstream = self
            .upstream
            .execute_audio_transcription(&provider, &raw_model, &normalized)
            .await?;
        let attempt = upstream.as_attempt(provider.id.clone(), candidate.clone());

        if (200..300).contains(&attempt.status) {
            self.repository.mark_provider_success(&provider.id).await?;
            let mut json_body: Value = serde_json::from_slice(&upstream.bytes)?;
            if let Some(map) = json_body.as_object_mut() {
                map.insert(
                    "_kou_router".to_string(),
                    serde_json::to_value(RoutingDebug {
                        requested_model,
                        resolved_model,
                        endpoint: EndpointKind::AudioTranscriptions.as_str().to_string(),
                        path_suffix: None,
                        tried: vec![attempt],
                    })?,
                );
            }
            return Ok(Json(json_body).into_response());
        }

        self.repository
            .mark_provider_failure(&provider.id, &String::from_utf8_lossy(&upstream.bytes))
            .await?;
        let should_fallback = fallback_error(upstream.status, &String::from_utf8_lossy(&upstream.bytes));
        if should_fallback {
            return Err(AppError::Upstream(format!(
                "audio transcriptions request failed for {} with fallback-worthy status {}",
                provider.provider, upstream.status
            )));
        }
        Err(AppError::Upstream(format!(
            "audio transcriptions request failed for {}",
            provider.provider
        )))
    }

    async fn eligible_providers(
        &self,
        endpoint: EndpointKind,
    ) -> AppResult<Vec<crate::models::ProviderConnection>> {
        let providers = self.repository.list_provider_connections().await?;
        let now = chrono::Utc::now();
        let providers: Vec<_> = providers
            .into_iter()
            .filter(|provider| {
                provider.enabled
                    && provider
                        .rate_limited_until
                        .map(|until| until < now)
                        .unwrap_or(true)
                    && provider
                        .circuit_open_until
                        .map(|until| until < now)
                        .unwrap_or(true)
                    && crate::models::supports_endpoint(&provider.supported_endpoints, endpoint)
            })
            .collect();
        if providers.is_empty() {
            return Err(AppError::BadRequest(format!(
                "no enabled providers configured for {}",
                endpoint.as_str()
            )));
        }
        Ok(providers)
    }
}

fn resolve_provider(
    providers: &[crate::models::ProviderConnection],
    resolved_model: &str,
    endpoint: EndpointKind,
) -> AppResult<(String, crate::models::ProviderConnection)> {
    let candidate = resolved_model.to_string();
    let (prefix, _) = split_model(&candidate)?;
    let mut matching: Vec<_> = providers
        .iter()
        .filter(|provider| provider.model_prefix == prefix || provider.provider == prefix)
        .cloned()
        .collect();
    matching.sort_by_key(|provider| provider.priority);
    matching
        .into_iter()
        .next()
        .map(|provider| (candidate, provider))
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "no provider found for model prefix and endpoint {}",
                endpoint.as_str()
            ))
        })
}

async fn collect_transcription_payload(mut multipart: Multipart) -> AppResult<AudioTranscriptionPayload> {
    let mut model = None;
    let mut filename = None;
    let mut content_type = None;
    let mut bytes = None;
    let mut text_fields = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| AppError::BadRequest(format!("invalid multipart form data: {err}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            filename = Some(field.file_name().unwrap_or("audio.wav").to_string());
            content_type = field.content_type().map(|v| v.to_string());
            bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|err| AppError::BadRequest(format!("failed to read uploaded file: {err}")))?
                    .to_vec(),
            );
        } else {
            let value = field
                .text()
                .await
                .map_err(|err| AppError::BadRequest(format!("failed to read multipart field {name}: {err}")))?;
            if name == "model" {
                model = Some(value.clone());
            }
            text_fields.push((name, value));
        }
    }

    let model = model.ok_or_else(|| AppError::BadRequest("missing model".into()))?;
    let bytes = bytes.ok_or_else(|| AppError::BadRequest("missing file".into()))?;
    Ok(AudioTranscriptionPayload {
        model,
        filename: filename.unwrap_or_else(|| "audio.wav".to_string()),
        content_type,
        bytes,
        text_fields,
    })
}

fn build_audio_response(upstream: AudioResponse, debug: RoutingDebug) -> AppResult<Response> {
    let mut response = Response::new(axum::body::Body::from(upstream.bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&upstream.content_type)
            .map_err(|err| AppError::BadRequest(format!("invalid audio content type: {err}")))?,
    );
    headers.insert(
        "x-kou-router-debug",
        HeaderValue::from_str(&serde_json::to_string(&debug)?)
            .map_err(|err| AppError::BadRequest(format!("debug header too large/invalid: {err}")))?,
    );
    Ok(response)
}

fn split_model(value: &str) -> AppResult<(String, String)> {
    let mut parts = value.splitn(2, '/');
    let provider = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("invalid model identifier: {value}")))?;
    let model = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("invalid model identifier: {value}")))?;
    Ok((provider.to_string(), model.to_string()))
}
