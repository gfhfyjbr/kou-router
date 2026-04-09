use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use http_body_util::BodyExt;
use kou_router::{build_app, init_db, routes::AppState, SqliteRepository};
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

/// Spawn a mock upstream that replies with the given status and body on all proxy endpoints.
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
            "/embeddings",
            post({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (status, Json(body)).into_response() }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr: SocketAddr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    format!("http://{}", addr)
}

async fn create_provider(state: &AppState, id: &str, base_url: String, priority: i64) {
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: id.to_string(),
            base_url,
            api_key: None,
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: None,
            stream_endpoint_paths: None,
            model_prefix: Some(id.to_string()),
            name: Some(id.to_string()),
            enabled: true,
            priority: Some(priority),
            default_model: Some(format!("{id}/default")),
            supported_endpoints: None,
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// 1-13: Normalization / validation errors — no upstream needed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_completions_missing_model() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({"messages": [{"role": "user", "content": "hi"}]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chat_completions_invalid_model_format() {
    // Bare model names (no slash) are now supported for Claude Code compatibility.
    // "no-slash" passes split_model returning ("", "no-slash") then
    // find_providers_for_bare_model tries to match. With only a "dummy" provider
    // and no known model prefix, all providers are tried and fail → 502 upstream error.
    let state = setup_state().await;
    create_provider(&state, "dummy", "http://127.0.0.1:1".into(), 0).await;
    let app = build_app(state);
    let resp = app
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({"model": "no-slash", "messages": [{"role": "user", "content": "hi"}]}),
        ))
        .await
        .unwrap();
    // Bare unknown models now attempt all providers — connection error or upstream failure
    let status = resp.status().as_u16();
    assert!(
        status == 500 || status == 502,
        "expected 500 or 502, got {status}"
    );
}

#[tokio::test]
async fn test_embeddings_missing_input() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/embeddings",
            json!({"model": "p1/embed"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_images_missing_prompt() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/images/generations",
            json!({"model": "p1/img"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_music_missing_prompt() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/music/generations",
            json!({"model": "p1/music"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_video_missing_prompt() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/videos/generations",
            json!({"model": "p1/video"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_moderations_missing_input() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json("/v1/moderations", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_rerank_missing_query() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/rerank",
            json!({"model": "p1/rerank", "documents": ["a"]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_rerank_missing_documents() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/rerank",
            json!({"model": "p1/rerank", "query": "test"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_search_missing_query() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json("/v1/search", json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_audio_speech_missing_input() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/audio/speech",
            json!({"model": "p1/tts"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_ollama_chat_missing_messages() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(post_json(
            "/v1/api/chat",
            json!({"model": "p1/llama"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_invalid_json_body() {
    let app = build_app(setup_state().await);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    // axum's Json extractor returns 422 for parse failures
    assert_ne!(resp.status(), StatusCode::OK);
    assert!(
        resp.status() == StatusCode::UNPROCESSABLE_ENTITY
            || resp.status() == StatusCode::BAD_REQUEST,
        "expected 422 or 400, got {}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// 14-16: Upstream routing errors — need mock servers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_non_retriable_error_stops_fallback() {
    // Two providers sharing prefix "p1". First (priority 0) returns 400 (non-retriable).
    // The break in the inner loop prevents trying the second provider (priority 1).
    let bad_url = spawn_mock_server(StatusCode::BAD_REQUEST, json!({"error": "bad"})).await;
    let good_url = spawn_mock_server(
        StatusCode::OK,
        json!({
            "id": "chatcmpl-good",
            "object": "chat.completion",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
        }),
    )
    .await;

    let state = setup_state().await;
    // Both providers share the same model_prefix "p1"
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "p1a".to_string(),
            base_url: bad_url,
            api_key: None,
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: None,
            stream_endpoint_paths: None,
            model_prefix: Some("p1".to_string()),
            name: Some("BadProvider".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("p1/test-model".to_string()),
            supported_endpoints: None,
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();
    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "p1b".to_string(),
            base_url: good_url,
            api_key: None,
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: None,
            stream_endpoint_paths: None,
            model_prefix: Some("p1".to_string()),
            name: Some("GoodProvider".to_string()),
            enabled: true,
            priority: Some(1),
            default_model: Some("p1/test-model".to_string()),
            supported_endpoints: None,
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({"model": "p1/test-model", "messages": [{"role": "user", "content": "hi"}]}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("all providers"),
        "expected upstream error, got: {payload}"
    );
}

#[tokio::test]
async fn test_all_providers_fail_returns_502() {
    let fail1 = spawn_mock_server(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({"error": "boom1"}),
    )
    .await;
    let fail2 = spawn_mock_server(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({"error": "boom2"}),
    )
    .await;

    let state = setup_state().await;
    create_provider(&state, "p1", fail1, 0).await;
    create_provider(&state, "p2", fail2, 0).await;
    state
        .repository
        .create_combo(kou_router::models::NewCombo {
            name: "fail-combo".to_string(),
            strategy: kou_router::models::ComboStrategy::Priority,
            models: vec!["p1/model".to_string(), "p2/model".to_string()],
            enabled: true,
        })
        .await
        .unwrap();

    let app = build_app(state);
    let resp = app
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({"model": "fail-combo", "messages": [{"role": "user", "content": "hi"}]}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn test_no_providers_for_model_prefix() {
    let state = setup_state().await;
    // Create a provider with a different prefix so the "no enabled providers" check passes
    create_provider(&state, "other", "http://127.0.0.1:1".into(), 0).await;

    let app = build_app(state);
    let resp = app
        .oneshot(post_json(
            "/v1/chat/completions",
            json!({"model": "unknown/model", "messages": [{"role": "user", "content": "hi"}]}),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let msg = payload["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("all providers") || msg.contains("failed"),
        "expected routing failure, got: {msg}"
    );
}
