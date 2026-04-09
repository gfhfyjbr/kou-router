use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use http_body_util::BodyExt;
use kou_router::{build_app, init_db, models::OpenAiModelsResponse, routes::AppState, SqliteRepository};
use reqwest::multipart;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn setup_state() -> AppState {
    let database_url = format!(
        "sqlite:file:krt-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let pool = init_db(&database_url).await.expect("db init");
    let repo = Arc::new(SqliteRepository::new(pool));
    AppState::new(repo)
}

async fn spawn_mock_server(status: StatusCode, body: Value) -> String {
    let app = Router::new()
        .route(
            "/chat/completions",
            post({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, Json(body)).into_response() }
                }
            }),
        )
        .route(
            "/messages",
            post({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, Json(body)).into_response() }
                }
            }),
        )
        .route(
            "/responses",
            post({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, Json(body)).into_response() }
                }
            }),
        )
        .route(
            "/responses/native/path",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": "resp-native",
                        "object": "response",
                        "output": [{
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": "suffix-ok"}]
                        }]
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/api/chat",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": "chatcmpl-ollama",
                        "object": "chat.completion",
                        "model": "ollama-model",
                        "choices": [{
                            "index": 0,
                            "message": {"role": "assistant", "content": "ollama-hi"},
                            "finish_reason": "stop"
                        }]
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/embeddings",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "object": "list",
                        "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2, 0.3]}],
                        "model": "embed-small",
                        "usage": {"prompt_tokens": 3, "total_tokens": 3}
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/images/generations",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "created": 1700000000,
                        "data": [{"url": "https://example.com/image.png"}]
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/music/generations",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "created": 1700000001,
                        "data": [{"b64_json": "RkFLRV9XQVY=", "format": "wav"}]
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/videos/generations",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "created": 1700000002,
                        "data": [{"b64_json": "RkFLRV9NUDQ=", "format": "mp4"}]
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/moderations",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": "modr-1",
                        "model": "omni-moderation-latest",
                        "results": [{
                            "flagged": false,
                            "categories": {"violence": false, "self-harm": false},
                            "category_scores": {"violence": 0.01, "self-harm": 0.0}
                        }]
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/rerank",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": "rerank-1",
                        "results": [
                            {"index": 1, "relevance_score": 0.98},
                            {"index": 0, "relevance_score": 0.72}
                        ],
                        "meta": {"api_version": {"version": "2"}}
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/search",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "provider": "web",
                        "query": "rust async router",
                        "results": [
                            {
                                "title": "Rust async guide",
                                "url": "https://example.com/rust-async",
                                "snippet": "async networking in rust"
                            }
                        ],
                        "usage": {"queries_used": 1, "search_cost_usd": 0.001}
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/stream/chat/completions",
            post(|| async move {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: [DONE]\n\n",
                )
                    .into_response()
            }),
        )
        .route(
            "/audio/speech",
            post(|| async move {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "audio/mpeg")],
                    "FAKE_MP3_AUDIO",
                )
                    .into_response()
            }),
        )
        .route(
            "/audio/transcriptions",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "text": "transcribed hello world"
                    })),
                )
                    .into_response()
            }),
        )
        .route("/health", get(|| async { Json(json!({ "ok": true })) }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let addr: SocketAddr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock app");
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn health_endpoint_works() {
    let app = build_app(setup_state().await);
    let response = app
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn models_include_providers_combos_and_aliases_management_is_persistent() {
    let state = setup_state().await;
    let app = build_app(state.clone());

    let create_provider = Request::builder()
        .method("POST")
        .uri("/api/providers")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "provider": "mock-openai",
                "base_url": "http://127.0.0.1:9",
                "model_prefix": "mo",
                "default_model": "mo/gpt-4o-mini"
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(create_provider).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let create_combo = Request::builder()
        .method("POST")
        .uri("/api/combos")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "name": "coding-pack",
                "strategy": "priority",
                "models": ["mo/gpt-4o-mini"]
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(create_combo).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let create_alias = Request::builder()
        .method("POST")
        .uri("/api/models/alias")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "alias": "fast-code",
                "target": "mo/gpt-4o-mini"
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(create_alias).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let settings_upsert = Request::builder()
        .method("POST")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"requireAuthForModels": true, "globalFallbackModel": "mo/gpt-4o-mini"})
                .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(settings_upsert).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(Request::builder().uri("/v1/models").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let catalog: OpenAiModelsResponse = serde_json::from_slice(&body).unwrap();
    let ids: Vec<_> = catalog.data.into_iter().map(|model| model.id).collect();

    assert!(ids.contains(&"coding-pack".to_string()));
    assert!(ids.contains(&"mo/gpt-4o-mini".to_string()));

    let aliases_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/models/alias")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(aliases_response.status(), StatusCode::OK);
    let alias_body = aliases_response.into_body().collect().await.unwrap().to_bytes();
    let aliases: Vec<Value> = serde_json::from_slice(&alias_body).unwrap();
    assert_eq!(aliases[0]["alias"], "fast-code");
    assert_eq!(aliases[0]["target"], "mo/gpt-4o-mini");

    let settings_response = app
        .oneshot(Request::builder().uri("/api/settings").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(settings_response.status(), StatusCode::OK);
    let settings_body = settings_response.into_body().collect().await.unwrap().to_bytes();
    let settings: Value = serde_json::from_slice(&settings_body).unwrap();
    assert_eq!(settings["requireAuthForModels"], true);
    assert_eq!(settings["globalFallbackModel"], "mo/gpt-4o-mini");
}

#[tokio::test]
async fn provider_presets_can_be_listed_and_imported() {
    let state = setup_state().await;
    let app = build_app(state);

    let presets = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/providers/presets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(presets.status(), StatusCode::OK);
    let body = presets.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let presets = payload.as_array().unwrap();
    assert!(presets.iter().any(|preset| preset["id"] == "openai"));
    assert!(presets.iter().any(|preset| preset["id"] == "anthropic"));
    assert!(presets.iter().any(|preset| preset["id"] == "antigravity"));
    assert!(presets.iter().any(|preset| preset["id"] == "serper-search"));

    let imported = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/providers/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "preset_id": "anthropic",
                        "api_key": "sk-ant-123",
                        "model_prefix": "anthro",
                        "name": "Anthropic Imported",
                        "priority": 2,
                        "rate_limit_protection": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(imported.status(), StatusCode::OK);
    let body = imported.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["provider"], "anthropic");
    assert_eq!(payload["auth_header"], "x-api-key");
    assert_eq!(payload["extra_headers"]["Anthropic-Version"], "2023-06-01");
    assert_eq!(payload["endpoint_paths"]["messages"], "/messages");
    assert_eq!(payload["model_prefix"], "anthro");
    assert_eq!(payload["default_model"], "anthropic/claude-sonnet-4.6");
    assert_eq!(payload["supported_endpoints"][0], "messages");
    assert_eq!(payload["rate_limit_protection"], true);

    let providers = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(providers.status(), StatusCode::OK);
    let body = providers.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let providers = payload.as_array().unwrap();
    let imported = providers
        .iter()
        .find(|provider| provider["provider"] == "anthropic")
        .unwrap();
    assert_eq!(imported["name"], "Anthropic Imported");
    assert_eq!(imported["auth_type"], "apikey");
    assert_eq!(imported["auth_header"], "x-api-key");
}

#[tokio::test]
async fn model_and_admin_listing_routes_are_supported() {
    let state = setup_state().await;
    let app = build_app(state.clone());

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/providers")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider": "admin-openai",
                        "base_url": "http://127.0.0.1:9",
                        "model_prefix": "ao",
                        "default_model": "ao/gpt-4.1-mini",
                        "name": "Admin Provider"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/combos")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "admin-pack",
                        "strategy": "priority",
                        "models": ["ao/gpt-4.1-mini"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let v1_models = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v1_models.status(), StatusCode::OK);
    let body = v1_models.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let ids = payload["data"].as_array().unwrap();
    assert!(ids.iter().any(|item| item["id"] == "admin-pack"));
    assert!(ids.iter().any(|item| item["id"] == "ao/gpt-4.1-mini"));

    let providers = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(providers.status(), StatusCode::OK);
    let body = providers.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(payload.as_array().unwrap().iter().any(|item| item["provider"] == "admin-openai"));

    let combos = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/combos")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(combos.status(), StatusCode::OK);
    let body = combos.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(payload.as_array().unwrap().iter().any(|item| item["name"] == "admin-pack"));
}

#[tokio::test]
async fn chat_completions_fallbacks_to_next_combo_target() {
    let first = spawn_mock_server(
        StatusCode::TOO_MANY_REQUESTS,
        json!({"error": {"message": "rate limit exceeded"}}),
    )
    .await;
    let second = spawn_mock_server(
        StatusCode::OK,
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello from rust port"},
                "finish_reason": "stop"
            }]
        }),
    )
    .await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "p1".to_string(), base_url: first, api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("p1".to_string()), name: Some("First".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("p1/fail-model".to_string()),
        supported_endpoints: None,
        rate_limit_protection: true, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "p2".to_string(), base_url: second, api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("p2".to_string()), name: Some("Second".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("p2/success-model".to_string()),
        supported_endpoints: None,
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_combo(kou_router::models::NewCombo {
            name: "coding-pack".to_string(),
            strategy: kou_router::models::ComboStrategy::Priority,
            models: vec!["p1/fail-model".to_string(), "p2/success-model".to_string()],
            enabled: true,
        })
        .await
        .unwrap();

    let app = build_app(state.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "coding-pack",
                        "messages": [{"role": "user", "content": "ping"}],
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["choices"][0]["message"]["content"], "hello from rust port");
    assert_eq!(payload["_kou_router"]["requested_model"], "coding-pack");
    let tried = payload["_kou_router"]["tried"].as_array().unwrap();
    assert_eq!(tried.len(), 2);
    assert_eq!(tried[0]["status"], 429);
    assert_eq!(tried[1]["status"], 200);

    let providers = state.repository.list_provider_connections().await.unwrap();
    let failed = providers.iter().find(|provider| provider.provider == "p1").unwrap();
    let succeeded = providers.iter().find(|provider| provider.provider == "p2").unwrap();
    assert_eq!(failed.test_status.as_deref(), Some("error"));
    assert_eq!(failed.last_error_type.as_deref(), Some("rate_limit"));
    assert!(failed.rate_limited_until.is_some());
    assert!(failed.circuit_open_until.is_some());
    assert!(failed.backoff_level >= 1);
    assert_eq!(succeeded.test_status.as_deref(), Some("ok"));
    assert!(succeeded.last_used_at.is_some());
    assert!(succeeded.consecutive_use_count >= 1);
}

#[tokio::test]
async fn compatibility_endpoints_are_supported() {
    let upstream = spawn_mock_server(
        StatusCode::OK,
        json!({
            "id": "chatcmpl-compat",
            "object": "chat.completion",
            "model": "p1/compat-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "compat-hi"},
                "finish_reason": "stop"
            }]
        }),
    )
    .await;
    let stream_upstream = format!("{}/stream", upstream);

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "p1".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("p1".to_string()), name: Some("Compat".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("p1/compat-model".to_string()),
        supported_endpoints: None,
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "ps".to_string(), base_url: stream_upstream, api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("ps".to_string()), name: Some("Stream".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("ps/stream-model".to_string()),
        supported_endpoints: None,
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();

    let app = build_app(state);

    let responses = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "p1/compat-model",
                        "input": "say hi"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(responses.status(), StatusCode::OK);
    let body = responses.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["object"], "response");
    assert_eq!(payload["output"][0]["content"][0]["text"], "compat-hi");

    let responses_suffix = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/responses/native/path")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "p1/compat-model",
                        "input": "native"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(responses_suffix.status(), StatusCode::OK);
    let body = responses_suffix.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["output"][0]["content"][0]["text"], "suffix-ok");
    assert_eq!(payload["_kou_router"]["path_suffix"], "native/path");

    let messages = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "p1/compat-model",
                        "messages": [{"role": "user", "content": "yo"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(messages.status(), StatusCode::OK);
    let body = messages.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["type"], "message");
    assert_eq!(payload["content"][0]["text"], "compat-hi");

    let completions = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "p1/compat-model",
                        "prompt": "legacy prompt"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(completions.status(), StatusCode::OK);
    let body = completions.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["choices"][0]["message"]["content"], "compat-hi");

    let ollama = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/api/chat")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "p1/compat-model",
                        "messages": [{"role": "user", "content": "yo"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ollama.status(), StatusCode::OK);
    let body = ollama.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["message"]["content"], "ollama-hi");

    let stream = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "ps/stream-model",
                        "messages": [{"role": "user", "content": "stream pls"}],
                        "stream": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(stream.headers().get("content-type").unwrap(), "text/event-stream");
    let body = stream.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("data:"));
    assert!(text.contains("[DONE]"));
}

#[tokio::test]
async fn embeddings_and_images_endpoints_are_supported() {
    let upstream = spawn_mock_server(StatusCode::OK, json!({"ok": true})).await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "embedder".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("embedder".to_string()), name: Some("Embeddings".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("embedder/text-embedding-3-small-1536".to_string()),
        supported_endpoints: Some(vec!["embeddings".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "imager".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("imager".to_string()), name: Some("Images".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("imager/dall-e-3".to_string()),
        supported_endpoints: Some(vec!["images".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();

    let app = build_app(state);

    let embedding_catalog = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/embeddings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(embedding_catalog.status(), StatusCode::OK);
    let body = embedding_catalog.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["id"], "embedder/text-embedding-3-small-1536");
    assert_eq!(payload["data"][0]["type"], "embeddings");
    assert_eq!(payload["data"][0]["dimensions"], 1536);

    let image_catalog = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/images/generations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(image_catalog.status(), StatusCode::OK);
    let body = image_catalog.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["id"], "imager/dall-e-3");
    assert_eq!(payload["data"][0]["type"], "images");
    assert_eq!(payload["data"][0]["supported_sizes"][0], "256x256");

    let embeddings = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/embeddings")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "embedder/text-embedding-3-small-1536",
                        "input": ["hello", "world"],
                        "dimensions": 1536
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(embeddings.status(), StatusCode::OK);
    let body = embeddings.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["object"], "embedding");
    assert_eq!(payload["_kou_router"]["endpoint"], "embeddings");

    let images = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/images/generations")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "imager/dall-e-3",
                        "prompt": "draw a rust crab spaceship",
                        "size": "1024x1024",
                        "n": 1
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(images.status(), StatusCode::OK);
    let body = images.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["url"], "https://example.com/image.png");
    assert_eq!(payload["_kou_router"]["endpoint"], "images.generations");
}

#[tokio::test]
async fn music_and_video_endpoints_are_supported() {
    let upstream = spawn_mock_server(StatusCode::OK, json!({"ok": true})).await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "composer".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("composer".to_string()), name: Some("Music".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("composer/musicgen-medium".to_string()),
        supported_endpoints: Some(vec!["music".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "director".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("director".to_string()), name: Some("Video".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("director/animatediff".to_string()),
        supported_endpoints: Some(vec!["video".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();

    let app = build_app(state);

    let music_catalog = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/music/generations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(music_catalog.status(), StatusCode::OK);
    let body = music_catalog.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["id"], "composer/musicgen-medium");
    assert_eq!(payload["data"][0]["type"], "music");
    assert_eq!(payload["data"][0]["supported_sizes"][0], "wav");

    let video_catalog = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/videos/generations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(video_catalog.status(), StatusCode::OK);
    let body = video_catalog.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["id"], "director/animatediff");
    assert_eq!(payload["data"][0]["type"], "video");
    assert_eq!(payload["data"][0]["supported_sizes"][0], "mp4");

    let music = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/music/generations")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "composer/musicgen-medium",
                        "prompt": "ambient synthwave for late-night coding",
                        "duration": 12
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(music.status(), StatusCode::OK);
    let body = music.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["format"], "wav");
    assert_eq!(payload["_kou_router"]["endpoint"], "music.generations");

    let video = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/videos/generations")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "director/animatediff",
                        "prompt": "a rust crab piloting a neon spaceship",
                        "size": "512x512",
                        "frames": 16
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(video.status(), StatusCode::OK);
    let body = video.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["format"], "mp4");
    assert_eq!(payload["_kou_router"]["endpoint"], "videos.generations");
}

#[tokio::test]
async fn moderations_rerank_and_search_endpoints_are_supported() {
    let upstream = spawn_mock_server(StatusCode::OK, json!({"ok": true})).await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "guard".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("guard".to_string()), name: Some("Moderation".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("guard/omni-moderation-latest".to_string()),
        supported_endpoints: Some(vec!["moderations".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "ranker".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("ranker".to_string()), name: Some("Rerank".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("ranker/rerank-v1".to_string()),
        supported_endpoints: Some(vec!["rerank".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "search".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("search".to_string()), name: Some("Search".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("search/web".to_string()),
        supported_endpoints: Some(vec!["search".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();

    let app = build_app(state);

    let search_catalog = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/search")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(search_catalog.status(), StatusCode::OK);
    let body = search_catalog.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["data"][0]["id"], "search/web");
    assert_eq!(payload["data"][0]["supported_sizes"][0], "web");

    let moderations = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/moderations")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "guard/omni-moderation-latest",
                        "input": "harmless text"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(moderations.status(), StatusCode::OK);
    let body = moderations.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["results"][0]["flagged"], false);
    assert_eq!(payload["_kou_router"]["endpoint"], "moderations");

    let rerank = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/rerank")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "ranker/rerank-v1",
                        "query": "best rust async runtime",
                        "documents": ["tokio", "axum", "actix"],
                        "top_n": 2
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rerank.status(), StatusCode::OK);
    let body = rerank.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["results"][0]["index"], 1);
    assert_eq!(payload["_kou_router"]["endpoint"], "rerank");

    let search = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "search/web",
                        "provider": "web",
                        "query": "rust async router",
                        "max_results": 5
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(search.status(), StatusCode::OK);
    let body = search.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["provider"], "search");
    assert_eq!(payload["upstream_provider"], "web");
    assert_eq!(payload["results"][0]["title"], "Rust async guide");
    assert_eq!(payload["_kou_router"]["endpoint"], "search");
    let state = setup_state().await;
    let search_app = Router::new()
        .route(
            "/search",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "organic": [
                            {
                                "title": "Serper Rust",
                                "link": "https://example.com/serper-rust",
                                "snippet": "serper async result"
                            }
                        ],
                        "searchParameters": {"totalResults": 1}
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/res/v1/web/search",
            get(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "web": {
                            "results": [
                                {
                                    "title": "Brave Rust",
                                    "url": "https://example.com/brave-rust",
                                    "description": "brave async result",
                                    "meta_url": {"favicon": "https://example.com/favicon.ico"}
                                }
                            ],
                            "totalCount": 1
                        }
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/search-exa",
            post(|| async move {
                (
                    StatusCode::OK,
                    Json(json!({
                        "results": [
                            {
                                "title": "Exa Rust",
                                "url": "https://example.com/exa-rust",
                                "highlights": ["exa async highlight"],
                                "score": 0.91,
                                "publishedDate": "2026-04-04T00:00:00Z",
                                "favicon": "https://example.com/exa.ico",
                                "author": "Exa Bot",
                                "image": "https://example.com/exa.png",
                                "text": "exa full text body"
                            }
                        ]
                    })),
                )
                    .into_response()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, search_app).await.unwrap();
    });
    let upstream = format!("http://{}", addr);

    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "serper-search".to_string(),
            base_url: upstream.clone(),
            api_key: Some("serper-key".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "x-api-key".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: Some(BTreeMap::from([("search".to_string(), "/search".to_string())])),
            stream_endpoint_paths: None,
            model_prefix: Some("serper-search".to_string()),
            name: Some("Serper Search".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("serper-search/web".to_string()),
            supported_endpoints: Some(vec!["search".to_string()]),
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "brave-search".to_string(),
            base_url: upstream.clone(),
            api_key: Some("brave-key".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "x-subscription-token".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: Some(BTreeMap::from([(
                "search".to_string(),
                format!("{}/res/v1/web/search", upstream),
            )])),
            stream_endpoint_paths: None,
            model_prefix: Some("brave-search".to_string()),
            name: Some("Brave Search".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("brave-search/web".to_string()),
            supported_endpoints: Some(vec!["search".to_string()]),
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "exa-search".to_string(),
            base_url: upstream.clone(),
            api_key: Some("exa-key".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "x-api-key".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: Some(BTreeMap::from([("search".to_string(), format!("{}/search-exa", upstream))])),
            stream_endpoint_paths: None,
            model_prefix: Some("exa-search".to_string()),
            name: Some("Exa Search".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("exa-search/web".to_string()),
            supported_endpoints: Some(vec!["search".to_string()]),
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();
    let search_app = build_app(state);

    let serper = search_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "serper-search/web",
                        "query": "rust async router",
                        "search_type": "web",
                        "max_results": 3
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(serper.status(), StatusCode::OK);
    let body = serper.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["provider"], "serper-search");
    assert_eq!(payload["results"][0]["title"], "Serper Rust");
    assert_eq!(payload["results"][0]["url"], "https://example.com/serper-rust");

    let brave = search_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "brave-search/web",
                        "query": "rust async router",
                        "search_type": "web",
                        "max_results": 3
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(brave.status(), StatusCode::OK);
    let body = brave.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["provider"], "brave-search");
    assert_eq!(payload["results"][0]["title"], "Brave Rust");
    assert_eq!(payload["results"][0]["favicon_url"], "https://example.com/favicon.ico");

    let exa = search_app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "exa-search/web",
                        "query": "rust async router",
                        "search_type": "web",
                        "max_results": 3
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(exa.status(), StatusCode::OK);
    let body = exa.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["provider"], "exa-search");
    assert_eq!(payload["results"][0]["title"], "Exa Rust");
    assert_eq!(payload["results"][0]["content"]["text"], "exa full text body");
    assert_eq!(payload["results"][0]["metadata"]["author"], "Exa Bot");
}

#[tokio::test]
async fn audio_endpoints_are_supported() {
    let upstream = spawn_mock_server(StatusCode::OK, json!({"ok": true})).await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "speaker".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("speaker".to_string()), name: Some("Speech".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("speaker/tts-1".to_string()),
        supported_endpoints: Some(vec!["audio.speech".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection { provider: "scribe".to_string(), base_url: upstream.clone(), api_key: None, auth_type: "apikey".to_string(), auth_header: "bearer".to_string(), auth_prefix: None, extra_headers: BTreeMap::new(), endpoint_paths: None, stream_endpoint_paths: None, model_prefix: Some("scribe".to_string()), name: Some("Transcriptions".to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some("scribe/whisper-1".to_string()),
        supported_endpoints: Some(vec!["audio.transcriptions".to_string()]),
        rate_limit_protection: false, protocol_format: None, })
        .await
        .unwrap();

    let app = build_app(state.clone());

    let speech = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/audio/speech")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "speaker/tts-1",
                        "input": "say hello from rust",
                        "voice": "alloy",
                        "response_format": "mp3"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(speech.status(), StatusCode::OK);
    assert_eq!(speech.headers().get("content-type").unwrap(), "audio/mpeg");
    assert!(speech.headers().get("x-kou-router-debug").is_some());
    let body = speech.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(String::from_utf8(body.to_vec()).unwrap(), "FAKE_MP3_AUDIO");

    let client = reqwest::Client::new();
    let form = multipart::Form::new()
        .text("model", "scribe/whisper-1")
        .text("language", "en")
        .part(
            "file",
            multipart::Part::bytes(b"fake wav bytes".to_vec())
                .file_name("sample.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );
    let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let app_clone = app.clone();
    tokio::spawn(async move {
        axum::serve(server, app_clone).await.unwrap();
    });

    let response = client
        .post(format!("http://{}/v1/audio/transcriptions", addr))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = response.json().await.unwrap();
    assert_eq!(payload["text"], "transcribed hello world");
    assert_eq!(payload["_kou_router"]["endpoint"], "audio.transcriptions");

    let providers = state.repository.list_provider_connections().await.unwrap();
    let speaker = providers.iter().find(|provider| provider.provider == "speaker").unwrap();
    let scribe = providers.iter().find(|provider| provider.provider == "scribe").unwrap();
    assert_eq!(speaker.test_status.as_deref(), Some("ok"));
    assert_eq!(scribe.test_status.as_deref(), Some("ok"));
}


#[tokio::test]
async fn provider_auth_headers_and_path_overrides_are_supported() {
    let app = Router::new()
        .route(
            "/custom/messages",
            post(|headers: axum::http::HeaderMap| async move {
                let auth = headers
                    .get("x-api-key")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                let trace = headers
                    .get("x-trace-id")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": "msg-custom",
                        "object": "chat.completion",
                        "choices": [{
                            "index": 0,
                            "message": {"role": "assistant", "content": format!("auth={auth};trace={trace}")},
                            "finish_reason": "stop"
                        }]
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/custom/search",
            post(|headers: axum::http::HeaderMap| async move {
                let token = headers
                    .get("x-api-key")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                (
                    StatusCode::OK,
                    Json(json!({
                        "provider": "serper-search",
                        "query": "router headers",
                        "auth_seen": token,
                        "results": []
                    })),
                )
                    .into_response()
            }),
        )
        .route(
            "/custom/stream/chat/completions",
            post(|| async move {
                (
                    StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    "data: {\"choices\":[{\"delta\":{\"content\":\"override\"}}]}\n\ndata: [DONE]\n\n",
                )
                    .into_response()
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let upstream = format!("http://{}", addr);

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "anthro".to_string(),
            base_url: upstream.clone(),
            api_key: Some("anthropic-secret".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "x-api-key".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::from([("x-trace-id".to_string(), "trace-123".to_string())]),
            endpoint_paths: Some(BTreeMap::from([("messages".to_string(), "/custom/messages".to_string())])),
            stream_endpoint_paths: None,
            model_prefix: Some("anthro".to_string()),
            name: Some("Anthropic-style".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("anthro/claude-sonnet".to_string()),
            supported_endpoints: Some(vec!["messages".to_string()]),
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "searchx".to_string(),
            base_url: upstream.clone(),
            api_key: Some("search-secret".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "x-api-key".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: Some(BTreeMap::from([("search".to_string(), "/custom/search".to_string())])),
            stream_endpoint_paths: None,
            model_prefix: Some("searchx".to_string()),
            name: Some("Search Header".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("searchx/custom-search".to_string()),
            supported_endpoints: Some(vec!["search".to_string()]),
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "streamx".to_string(),
            base_url: upstream,
            api_key: Some("stream-secret".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: Some("Bearer".to_string()),
            extra_headers: BTreeMap::new(),
            endpoint_paths: None,
            stream_endpoint_paths: Some(BTreeMap::from([(
                "chat".to_string(),
                "/custom/stream/chat/completions".to_string(),
            )])),
            model_prefix: Some("streamx".to_string()),
            name: Some("Stream Override".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("streamx/model".to_string()),
            supported_endpoints: Some(vec!["chat".to_string()]),
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();

    let app = build_app(state);

    let messages = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "anthro/claude-sonnet",
                        "messages": [{"role": "user", "content": "ping"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(messages.status(), StatusCode::OK);
    let body = messages.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["content"][0]["text"], "auth=anthropic-secret;trace=trace-123");

    let search = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "searchx/custom-search",
                        "query": "router headers"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(search.status(), StatusCode::OK);
    let body = search.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["auth_seen"], "search-secret");
    assert_eq!(payload["_kou_router"]["endpoint"], "search");

    let stream = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "streamx/model",
                        "messages": [{"role": "user", "content": "stream pls"}],
                        "stream": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(stream.headers().get("content-type").unwrap(), "text/event-stream");
    let body = stream.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("override"));
    assert!(text.contains("[DONE]"));
}