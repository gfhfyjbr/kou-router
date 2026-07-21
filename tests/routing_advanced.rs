use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::post,
};
use http_body_util::BodyExt;
use kou_router::{SqliteRepository, build_app, init_db, routes::AppState};
use serde_json::{Value, json};
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

/// Spawn a mock server whose `/chat/completions` returns a chat completion
/// with `content` set to `identifier`, so the caller can tell which server was hit.
async fn spawn_identifying_mock(identifier: &str) -> String {
    let body = json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion",
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": identifier},
            "finish_reason": "stop"
        }]
    });
    let app = Router::new().route(
        "/chat/completions",
        post({
            let body = body.clone();
            move || {
                let body = body.clone();
                async move { (StatusCode::OK, Json(body)).into_response() }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{}", addr)
}

async fn spawn_counting_mock(identifier: &str) -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let body = json!({
        "id": "chatcmpl-cache",
        "object": "chat.completion",
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": identifier},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}
    });
    let app = Router::new().route(
        "/chat/completions",
        post({
            let body = body.clone();
            let calls = calls.clone();
            move || {
                let body = body.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::OK, Json(body)).into_response()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{}", addr), calls)
}

/// Spawn a mock that serves both `/chat/completions` (chat) and `/embeddings`.
async fn spawn_multi_endpoint_mock(chat_content: &str) -> String {
    let chat_body = json!({
        "id": "chatcmpl-multi",
        "object": "chat.completion",
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": chat_content},
            "finish_reason": "stop"
        }]
    });
    let embed_body = json!({
        "object": "list",
        "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}],
        "model": "embed-mock",
        "usage": {"prompt_tokens": 2, "total_tokens": 2}
    });
    let app = Router::new()
        .route(
            "/chat/completions",
            post({
                let body = chat_body.clone();
                move || {
                    let body = body.clone();
                    async move { (StatusCode::OK, Json(body)).into_response() }
                }
            }),
        )
        .route(
            "/embeddings",
            post({
                let body = embed_body.clone();
                move || {
                    let body = body.clone();
                    async move { (StatusCode::OK, Json(body)).into_response() }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{}", addr)
}

/// Spawn a mock that returns a Claude Messages API response on `/messages`.
async fn spawn_claude_mock() -> String {
    let body = json!({
        "id": "msg_123",
        "type": "message",
        "role": "assistant",
        "model": "claude-3-sonnet",
        "content": [{"type": "text", "text": "translated-hi"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 3}
    });
    let app = Router::new().route(
        "/messages",
        post({
            let body = body.clone();
            move || {
                let body = body.clone();
                async move { (StatusCode::OK, Json(body)).into_response() }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{}", addr)
}

fn new_provider(
    provider: &str,
    base_url: String,
    prefix: &str,
    name: &str,
    default_model: &str,
) -> kou_router::models::NewProviderConnection {
    kou_router::models::NewProviderConnection {
        provider: provider.to_string(),
        base_url,
        api_key: None,
        auth_type: "apikey".to_string(),
        auth_header: "bearer".to_string(),
        auth_prefix: None,
        extra_headers: BTreeMap::new(),
        endpoint_paths: None,
        stream_endpoint_paths: None,
        model_prefix: Some(prefix.to_string()),
        name: Some(name.to_string()),
        enabled: true,
        priority: Some(0),
        default_model: Some(default_model.to_string()),
        supported_endpoints: None,
        rate_limit_protection: false,
        protocol_format: None,
    }
}

async fn chat_request(app: &Router, model: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": model,
                        "messages": [{"role": "user", "content": "ping"}],
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    (status, payload)
}

async fn cacheable_chat_request(app: &Router, model: &str) -> (StatusCode, Option<String>, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-kou-response-cache", "read-write")
                .header("x-kou-response-cache-ttl", "60s")
                .body(Body::from(
                    json!({
                        "model": model,
                        "messages": [{"role": "user", "content": "ping"}],
                        "temperature": 0,
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let cache_header = response
        .headers()
        .get("x-kou-cache")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    (status, cache_header, payload)
}

async fn explicitly_cacheable_chat_request(
    app: &Router,
    model: &str,
) -> (StatusCode, Option<String>, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-kou-response-cache", "read-write")
                .header("x-kou-response-cache-ttl", "60s")
                .header("x-kou-response-cache-allow-nondeterministic", "true")
                .body(Body::from(
                    json!({
                        "model": model,
                        "messages": [{"role": "user", "content": "ping"}],
                        "stream": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let cache_header = response
        .headers()
        .get("x-kou-cache")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    (status, cache_header, payload)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_round_robin_alternates() {
    let mock_a = spawn_identifying_mock("server-a").await;
    let mock_b = spawn_identifying_mock("server-b").await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(new_provider("pa", mock_a, "pa", "ProviderA", "pa/a"))
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(new_provider("pb", mock_b, "pb", "ProviderB", "pb/b"))
        .await
        .unwrap();
    state
        .repository
        .create_combo(kou_router::models::NewCombo {
            name: "rr-combo".to_string(),
            strategy: kou_router::models::ComboStrategy::RoundRobin,
            models: vec!["pa/a".to_string(), "pb/b".to_string()],
            enabled: true,
        })
        .await
        .unwrap();

    let app = build_app(state);

    // Request 1: round-robin index 0 → pa/a → "server-a"
    let (s1, p1) = chat_request(&app, "rr-combo").await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(p1["choices"][0]["message"]["content"], "server-a");
    assert_eq!(p1["_kou_router"]["tried"][0]["model"], "pa/a");

    // Request 2: round-robin index 1 → pb/b → "server-b"
    let (s2, p2) = chat_request(&app, "rr-combo").await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(p2["choices"][0]["message"]["content"], "server-b");
    assert_eq!(p2["_kou_router"]["tried"][0]["model"], "pb/b");

    // Request 3: wraps back to index 0 → pa/a → "server-a"
    let (s3, p3) = chat_request(&app, "rr-combo").await;
    assert_eq!(s3, StatusCode::OK);
    assert_eq!(p3["choices"][0]["message"]["content"], "server-a");
    assert_eq!(p3["_kou_router"]["tried"][0]["model"], "pa/a");
}

#[tokio::test]
async fn test_response_cache_opt_in_reuses_successful_response() {
    let (mock, calls) = spawn_counting_mock("cached-ok").await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(new_provider(
            "cache",
            mock,
            "cache",
            "CacheProvider",
            "cache/m",
        ))
        .await
        .unwrap();

    let app = build_app(state);
    let (status, cache_header, payload) = cacheable_chat_request(&app, "cache/m").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache_header.as_deref(), Some("MISS"));
    assert_eq!(payload["choices"][0]["message"]["content"], "cached-ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let (status, cache_header, payload) = cacheable_chat_request(&app, "cache/m").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache_header.as_deref(), Some("HIT"));
    assert_eq!(payload["choices"][0]["message"]["content"], "cached-ok");
    assert_eq!(
        payload["_kou_router"]["tried"][0]["body"],
        "[response-cache hit]"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_response_cache_explicit_allow_reuses_chat_without_temperature() {
    let (mock, calls) = spawn_counting_mock("explicit-cached-ok").await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(new_provider(
            "cache",
            mock,
            "cache",
            "CacheProvider",
            "cache/m",
        ))
        .await
        .unwrap();

    let app = build_app(state);
    let (status, cache_header, payload) = explicitly_cacheable_chat_request(&app, "cache/m").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache_header.as_deref(), Some("MISS"));
    assert_eq!(
        payload["choices"][0]["message"]["content"],
        "explicit-cached-ok"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let (status, cache_header, payload) = explicitly_cacheable_chat_request(&app, "cache/m").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache_header.as_deref(), Some("HIT"));
    assert_eq!(
        payload["_kou_router"]["tried"][0]["body"],
        "[response-cache hit]"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_disabled_combo_error() {
    let mock = spawn_identifying_mock("nope").await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(new_provider("p1", mock, "p1", "P1", "p1/m"))
        .await
        .unwrap();
    state
        .repository
        .create_combo(kou_router::models::NewCombo {
            name: "disabled-combo".to_string(),
            strategy: kou_router::models::ComboStrategy::Priority,
            models: vec!["p1/m".to_string()],
            enabled: false,
        })
        .await
        .unwrap();

    let app = build_app(state);
    let (status, payload) = chat_request(&app, "disabled-combo").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = payload["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("disabled"),
        "expected 'disabled' in error, got: {msg}"
    );
}

#[tokio::test]
async fn test_alias_resolution_in_routing() {
    let mock = spawn_identifying_mock("alias-ok").await;

    let state = setup_state().await;
    state
        .repository
        .create_provider_connection(new_provider("p1", mock, "p1", "Provider1", "p1/real-model"))
        .await
        .unwrap();
    state
        .repository
        .upsert_alias("my-model", "p1/real-model")
        .await
        .unwrap();

    let app = build_app(state);
    let (status, payload) = chat_request(&app, "my-model").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["_kou_router"]["resolved_model"], "p1/real-model");
    assert_eq!(payload["_kou_router"]["requested_model"], "my-model");
    assert_eq!(payload["choices"][0]["message"]["content"], "alias-ok");
}

#[tokio::test]
async fn test_provider_disabled_not_selected() {
    let mock = spawn_identifying_mock("should-not-reach").await;

    let state = setup_state().await;
    let mut prov = new_provider("p1", mock, "p1", "DisabledProv", "p1/model");
    prov.enabled = false;
    state
        .repository
        .create_provider_connection(prov)
        .await
        .unwrap();

    let app = build_app(state);
    let (status, payload) = chat_request(&app, "p1/model").await;
    // No enabled provider → 400
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = payload["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("no enabled providers"),
        "expected 'no enabled providers' in error, got: {msg}"
    );
}

#[tokio::test]
async fn test_endpoint_filtering() {
    let mock = spawn_multi_endpoint_mock("chat-ok").await;

    let state = setup_state().await;
    let mut prov = new_provider("ep", mock, "ep", "EndpointFiltered", "ep/m");
    prov.supported_endpoints = Some(vec!["chat".to_string()]);
    state
        .repository
        .create_provider_connection(prov)
        .await
        .unwrap();

    let app = build_app(state);

    // Embeddings request should fail — provider only supports "chat"
    let embed_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/embeddings")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "ep/m",
                        "input": ["hello"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(embed_resp.status(), StatusCode::BAD_REQUEST);

    // Chat request should succeed
    let (status, payload) = chat_request(&app, "ep/m").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["choices"][0]["message"]["content"], "chat-ok");
}

#[tokio::test]
async fn test_provider_priority_ordering() {
    let mock_low = spawn_identifying_mock("low-priority").await;
    let mock_high = spawn_identifying_mock("high-priority").await;

    let state = setup_state().await;

    // priority=10 → lower priority (tried later)
    let mut prov_low = new_provider("prio", mock_low, "prio", "LowPri", "prio/m");
    prov_low.priority = Some(10);
    state
        .repository
        .create_provider_connection(prov_low)
        .await
        .unwrap();

    // priority=1 → higher priority (tried first)
    let mut prov_high = new_provider("prio", mock_high, "prio", "HighPri", "prio/m");
    prov_high.priority = Some(1);
    state
        .repository
        .create_provider_connection(prov_high)
        .await
        .unwrap();

    let app = build_app(state);
    let (status, payload) = chat_request(&app, "prio/m").await;
    assert_eq!(status, StatusCode::OK);
    // priority=1 tried first, succeeds immediately → content from high-priority mock
    assert_eq!(payload["choices"][0]["message"]["content"], "high-priority");
}

#[tokio::test]
async fn test_protocol_translation_claude_provider() {
    let mock = spawn_claude_mock().await;

    let state = setup_state().await;
    let mut prov = new_provider("cprov", mock, "cprov", "ClaudeProv", "cprov/sonnet");
    prov.protocol_format = Some("claude".to_string());
    // Route chat completions to /messages (like real Anthropic)
    let mut ep = BTreeMap::new();
    ep.insert("chat".to_string(), "/messages".to_string());
    prov.endpoint_paths = Some(ep);
    state
        .repository
        .create_provider_connection(prov)
        .await
        .unwrap();

    let app = build_app(state);

    // Send an OpenAI-format chat request
    let (status, payload) = chat_request(&app, "cprov/sonnet").await;
    assert_eq!(status, StatusCode::OK);

    // Response should be translated back to OpenAI format
    assert_eq!(payload["object"], "chat.completion");
    assert!(
        payload["id"].as_str().unwrap().contains("msg_123"),
        "id should contain original Claude id"
    );
    assert_eq!(payload["choices"][0]["message"]["content"], "translated-hi");
    assert_eq!(payload["choices"][0]["message"]["role"], "assistant");
    assert_eq!(payload["choices"][0]["finish_reason"], "stop");
    assert_eq!(payload["usage"]["prompt_tokens"], 5);
    assert_eq!(payload["usage"]["completion_tokens"], 3);
    assert_eq!(payload["usage"]["total_tokens"], 8);
}
