//! Integration tests for the Claude Code fingerprint pipeline.
//!
//! Each test spins up a tiny axum mock that captures the upstream HTTP request
//! the router makes and returns a canned response. The router is exercised end
//! to end through `app.oneshot(...)` so that the real provider-selection,
//! translation, fingerprint-injection, and OAuth-refresh paths are covered.
//!
//! All tests are marked `#[serial_test::serial]` because `ClaudeCodeFingerprint::new()`
//! reads process-wide env vars at `AppState::new` time. Some tests intentionally
//! mutate env (workload, custom headers, additional protection, OAuth token URL);
//! serializing every test prevents one test's env from polluting another's
//! `RouterService` initialization.

use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use chrono::{Duration as ChronoDuration, Utc};
use http_body_util::BodyExt;
use kou_router::{
    SqliteRepository, build_app, init_db,
    models::{
        NewProviderAccount, NewProviderConnection, ProviderAccount, ProviderAccountAuthMode,
        ProviderConnection,
    },
    routes::AppState,
};
use serde_json::{Value, json};
use serial_test::serial;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Mock upstream
// ============================================================================

#[derive(Debug, Clone)]
struct Captured {
    headers: HashMap<String, String>,
    body: Value,
}

#[derive(Clone)]
struct MockState {
    messages_calls: Arc<Mutex<Vec<Captured>>>,
    token_calls: Arc<Mutex<Vec<Value>>>,
    messages_responses: Arc<Mutex<Vec<(StatusCode, Value)>>>,
    token_responses: Arc<Mutex<Vec<(StatusCode, Value)>>>,
}

impl MockState {
    fn new(messages_responses: Vec<(StatusCode, Value)>) -> Self {
        Self {
            messages_calls: Arc::new(Mutex::new(Vec::new())),
            token_calls: Arc::new(Mutex::new(Vec::new())),
            messages_responses: Arc::new(Mutex::new(messages_responses)),
            token_responses: Arc::new(Mutex::new(vec![(
                StatusCode::OK,
                json!({
                    "access_token": "refreshed-tok",
                    "refresh_token": "new-rt",
                    "expires_in": 3600,
                }),
            )])),
        }
    }

    fn default_messages() -> Self {
        Self::new(vec![(StatusCode::OK, default_messages_response())])
    }

    fn captured(&self) -> Vec<Captured> {
        self.messages_calls.lock().unwrap().clone()
    }

    fn token_call_count(&self) -> usize {
        self.token_calls.lock().unwrap().len()
    }
}

fn default_messages_response() -> Value {
    json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-20250514",
        "content": [{"type": "text", "text": "hi"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 1}
    })
}

async fn capture_handler(
    State(state): State<MockState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let body_value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let header_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(n, v)| {
            v.to_str()
                .ok()
                .map(|v| (n.as_str().to_lowercase(), v.to_string()))
        })
        .collect();
    state.messages_calls.lock().unwrap().push(Captured {
        headers: header_map,
        body: body_value,
    });
    let mut q = state.messages_responses.lock().unwrap();
    let (status, body_json) = if q.len() > 1 {
        q.remove(0)
    } else {
        q.first()
            .cloned()
            .unwrap_or((StatusCode::OK, default_messages_response()))
    };
    (status, Json(body_json)).into_response()
}

async fn token_handler(State(state): State<MockState>, body: Bytes) -> impl IntoResponse {
    let body_value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    state.token_calls.lock().unwrap().push(body_value);
    let mut q = state.token_responses.lock().unwrap();
    let (status, body_json) = if q.len() > 1 {
        q.remove(0)
    } else {
        q.first().cloned().unwrap_or((
            StatusCode::OK,
            json!({
                "access_token": "refreshed-tok",
                "refresh_token": "new-rt",
                "expires_in": 3600,
            }),
        ))
    };
    (status, Json(body_json)).into_response()
}

/// Spawn a mock upstream that handles both Anthropic-style endpoints and the
/// OAuth token endpoint on the same host. Returns the base `http://host:port`.
async fn spawn_mock(state: MockState) -> String {
    let app = Router::new()
        .route("/messages", post(capture_handler))
        .route("/v1/messages", post(capture_handler))
        .route("/chat/completions", post(capture_handler))
        .route("/v1/chat/completions", post(capture_handler))
        .route("/v1/oauth/token", post(token_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    format!("http://{}", addr)
}

// ============================================================================
// AppState + provider helpers
// ============================================================================

async fn setup_state() -> AppState {
    let database_url = format!(
        "sqlite:file:fpi-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let pool = init_db(&database_url).await.expect("db init");
    let repo = Arc::new(SqliteRepository::new(pool));
    AppState::new(repo)
}

#[allow(clippy::too_many_arguments)]
async fn create_provider(
    state: &AppState,
    provider: &str,
    base_url: &str,
    api_key: Option<&str>,
    auth_header: &str,
    auth_prefix: Option<&str>,
    model_prefix: &str,
    default_model: &str,
    protocol_format: Option<&str>,
) -> ProviderConnection {
    state
        .repository
        .create_provider_connection(NewProviderConnection {
            provider: provider.to_string(),
            base_url: base_url.to_string(),
            api_key: api_key.map(str::to_string),
            auth_type: "apikey".to_string(),
            auth_header: auth_header.to_string(),
            auth_prefix: auth_prefix.map(str::to_string),
            extra_headers: BTreeMap::new(),
            endpoint_paths: Some(BTreeMap::new()),
            stream_endpoint_paths: Some(BTreeMap::new()),
            model_prefix: Some(model_prefix.to_string()),
            name: Some(format!("test-{provider}")),
            enabled: true,
            priority: Some(0),
            default_model: Some(default_model.to_string()),
            supported_endpoints: Some(vec!["messages".to_string(), "chat.completions".to_string()]),
            rate_limit_protection: false,
            protocol_format: protocol_format.map(str::to_string),
        })
        .await
        .expect("create provider")
}

async fn create_oauth_account(
    state: &AppState,
    provider_id: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: chrono::DateTime<Utc>,
    remote_account_id: Option<&str>,
) -> ProviderAccount {
    state
        .repository
        .create_provider_account(NewProviderAccount {
            provider_connection_id: provider_id.to_string(),
            label: Some("oauth-test".to_string()),
            auth_mode: ProviderAccountAuthMode::OAuth,
            api_key: None,
            access_token: Some(access_token.to_string()),
            refresh_token: Some(refresh_token.to_string()),
            expires_at: Some(expires_at),
            scopes: Some(vec!["user:inference".to_string()]),
            remote_account_id: remote_account_id.map(str::to_string),
            remote_email: None,
            is_fedramp: false,
            enabled: true,
            priority: Some(0),
            proxy_url: None,
        })
        .await
        .expect("create account")
}

fn messages_request_body(model: &str) -> Value {
    json!({
        "model": model,
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "Hello, world! This is a test message."}]
    })
}

fn chat_completions_request_body(model: &str) -> Value {
    json!({
        "model": model,
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "Hello, world! This is a chat test."}]
    })
}

async fn post_messages(
    app: axum::Router,
    body: Value,
    extra_headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut builder = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json");
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Body::from(body.to_string())).unwrap();
    app.oneshot(req).await.unwrap()
}

async fn post_chat_completions(
    app: axum::Router,
    body: Value,
    extra_headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut builder = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json");
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Body::from(body.to_string())).unwrap();
    app.oneshot(req).await.unwrap()
}

// ============================================================================
// Env guard (cleans up env on Drop, so panics don't leak state)
// ============================================================================

struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn new() -> Self {
        Self { keys: Vec::new() }
    }

    fn set(&mut self, key: &'static str, value: &str) -> &mut Self {
        unsafe {
            std::env::set_var(key, value);
        }
        self.keys.push(key);
        self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            unsafe {
                std::env::remove_var(key);
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Non-CC client sending an Anthropic 1P request gets the full
/// fingerprint stack injected (headers + body attribution + metadata).
#[tokio::test]
#[serial]
async fn test_non_cc_client_anthropic_apikey_injects_full_fingerprint() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(calls.len(), 1, "exactly one upstream call expected");
    let call = &calls[0];

    assert_eq!(call.headers.get("x-app").map(String::as_str), Some("cli"));

    let ua = call.headers.get("user-agent").expect("user-agent header");
    assert!(
        ua.starts_with("claude-cli/2.")
            && ua.contains("(external, cli")
            && ua.chars().nth(11).is_some_and(|c| c.is_ascii_digit()),
        "unexpected UA: {ua}"
    );

    let session_id = call
        .headers
        .get("x-claude-code-session-id")
        .expect("session id header");
    Uuid::parse_str(session_id).expect("session id is a valid UUID");

    let request_id = call
        .headers
        .get("x-client-request-id")
        .expect("client request id header");
    Uuid::parse_str(request_id).expect("client request id is a valid UUID");

    let beta = call.headers.get("anthropic-beta").expect("beta header");
    for token in [
        "claude-code-20250219",
        "interleaved-thinking-2025-05-14",
        "prompt-caching-scope-2026-01-05",
        "effort-2025-11-24",
    ] {
        assert!(beta.contains(token), "missing beta token {token} in {beta}");
    }
    for token in [
        "redact-thinking-2026-02-12",
        "fast-mode-2026-02-01",
        "task-budgets-2026-03-13",
        "advisor-tool-2026-03-01",
        "context-management-2025-06-27",
        "advanced-tool-use-2025-11-20",
        "structured-outputs-2025-12-15",
        "afk-mode-2026-01-31",
    ] {
        assert!(
            !beta.contains(token),
            "default beta set should not include {token}: {beta}"
        );
    }
    assert!(
        !beta.contains("context-1m-2025-08-07"),
        "context-1m beta must NOT be sent for pre-4.6 models without `[1m]`; got: {beta}"
    );

    let system = call
        .body
        .get("system")
        .and_then(Value::as_array)
        .expect("system should be array after fingerprint injection");
    let first_text = system[0]
        .get("text")
        .and_then(Value::as_str)
        .expect("first system block text");
    assert!(
        first_text.starts_with("x-anthropic-billing-header: cc_version="),
        "unexpected attribution: {first_text}"
    );

    let user_id_raw = call
        .body
        .get("metadata")
        .and_then(|m| m.get("user_id"))
        .and_then(Value::as_str)
        .expect("metadata.user_id should be a JSON string");
    let user_id: Value = serde_json::from_str(user_id_raw).expect("user_id parses as JSON");
    let device_id = user_id["device_id"].as_str().expect("device_id");
    assert_eq!(device_id.len(), 64, "device_id must be 64 hex chars");
    assert!(
        device_id.chars().all(|c| c.is_ascii_hexdigit()),
        "device_id must be hex: {device_id}"
    );
    assert_eq!(user_id["account_uuid"].as_str(), Some(""));
    let body_session = user_id["session_id"].as_str().expect("session_id");
    Uuid::parse_str(body_session).expect("body session_id is a valid UUID");
}

/// If the client supplies `anthropic-beta` but does not present itself as
/// Claude Code, the router still injects the missing Claude Code identity
/// headers while preserving the client-owned beta string exactly.
#[tokio::test]
#[serial]
async fn test_client_anthropic_beta_is_preserved_when_injecting_fingerprint() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-sonnet-4-20250514"),
        &[("anthropic-beta", "client-beta-1,client-beta-2")],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(
        call.headers.get("anthropic-beta").map(String::as_str),
        Some("client-beta-1,client-beta-2"),
        "router must not replace a client-supplied anthropic-beta"
    );
    assert_eq!(
        call.headers.get("x-app").map(String::as_str),
        Some("cli"),
        "missing Claude Code identity headers should still be injected"
    );
    assert!(
        call.body
            .get("system")
            .and_then(Value::as_array)
            .is_some_and(|system| system
                .first()
                .and_then(|block| block.get("text"))
                .and_then(Value::as_str)
                .is_some_and(|text| text.starts_with("x-anthropic-billing-header:"))),
        "body attribution should still be injected"
    );
}

/// OpenAI-compatible clients can hit `/v1/chat/completions`; when the selected
/// upstream speaks Claude, the translated request still receives Claude Code
/// headers.
#[tokio::test]
#[serial]
async fn test_openai_chat_completions_to_claude_provider_injects_claude_code_headers() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_chat_completions(
        app,
        chat_completions_request_body("anthropic/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    let status = resp.status();
    if status != StatusCode::OK {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        panic!(
            "expected OpenAI chat -> Claude request to succeed, got {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }

    let calls = mock.captured();
    assert_eq!(calls.len(), 1);
    let headers = &calls[0].headers;
    assert_eq!(headers.get("x-app").map(String::as_str), Some("cli"));
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    let beta = headers.get("anthropic-beta").expect("beta header");
    assert!(beta.contains("claude-code-20250219"), "beta: {beta}");
    assert!(
        beta.contains("prompt-caching-scope-2026-01-05"),
        "beta: {beta}"
    );
}

/// Test 2: When the client already presents itself as Claude Code (`x-app` set),
/// the router passes the supplied identity through verbatim and does NOT
/// inject its own fingerprint into headers or body.
#[tokio::test]
#[serial]
async fn test_cc_client_passes_headers_through_without_injection() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let client_session = Uuid::new_v4().to_string();
    let mut body = messages_request_body("anthropic/claude-sonnet-4-20250514");
    body["metadata"] = json!({"user_id": "client-supplied-id"});

    let resp = post_messages(
        app,
        body,
        &[
            ("x-app", "cli"),
            ("user-agent", "claude-cli/2.1.88 (external, cli)"),
            ("x-claude-code-session-id", client_session.as_str()),
        ],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];

    assert_eq!(
        call.headers
            .get("x-claude-code-session-id")
            .map(String::as_str),
        Some(client_session.as_str()),
        "router must passthrough the client-supplied session id"
    );

    let body_text = serde_json::to_string(&call.body).unwrap();
    assert!(
        !body_text.contains("x-anthropic-billing-header"),
        "no router-generated attribution should be injected when x-app already present: {body_text}"
    );
}

/// Test 3: A 3P provider (OpenRouter-style) running the Claude protocol
/// receives only the 3P-safe beta set and never the 1P-only betas.
#[tokio::test]
#[serial]
async fn test_3p_provider_no_first_party_betas() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "openrouter",
        &upstream,
        Some("or-test"),
        "x-api-key",
        None,
        "openrouter",
        "openrouter/anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("openrouter/anthropic/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(calls.len(), 1);
    let beta = calls[0].headers.get("anthropic-beta").expect("beta header");

    for forbidden in [
        "prompt-caching-scope",
        "fast-mode",
        "redact-thinking",
        "advanced-tool-use",
        "context-management-2025-06-27",
        "web-search-2025-03-05",
    ] {
        assert!(
            !beta.contains(forbidden),
            "3P provider must not receive {forbidden}: {beta}"
        );
    }
    assert!(
        beta.contains("tool-search-tool-2025-10-19"),
        "3P provider should get tool-search-tool beta: {beta}"
    );
}

/// Test 4: Vertex provider with Claude 4+ gets the `web-search-2025-03-05`
/// beta which is gated to Vertex.
#[tokio::test]
#[serial]
async fn test_vertex_claude4_includes_web_search() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "vertex",
        &upstream,
        Some("vertex-test"),
        "x-api-key",
        None,
        "vertex",
        "vertex/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("vertex/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(calls.len(), 1);
    let beta = calls[0].headers.get("anthropic-beta").expect("beta header");
    assert!(
        beta.contains("web-search-2025-03-05"),
        "Vertex + Claude4+ should receive web-search beta: {beta}"
    );
}

/// Test 5: Haiku (Claude 3.x) on 1P drops the claude-code, interleaved-thinking,
/// and context-1m betas, but keeps the universal `effort` beta and the 1P
/// `prompt-caching-scope` beta.
#[tokio::test]
#[serial]
async fn test_haiku_first_party_no_claude_code_no_interleaved_no_context1m() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-3-5-haiku-20241022",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-3-5-haiku-20241022"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    let beta = calls[0].headers.get("anthropic-beta").expect("beta header");
    for forbidden in [
        "claude-code-20250219",
        "interleaved-thinking-2025-05-14",
        "context-1m-2025-08-07",
    ] {
        assert!(
            !beta.contains(forbidden),
            "Haiku must not receive {forbidden}: {beta}"
        );
    }
    assert!(
        beta.contains("effort-2025-11-24"),
        "effort beta is universal: {beta}"
    );
    assert!(
        beta.contains("prompt-caching-scope-2026-01-05"),
        "1P prompt-caching-scope expected: {beta}"
    );
}

/// Test 6: Sonnet 4.5 on 1P uses the conservative default beta set.
#[tokio::test]
#[serial]
async fn test_claude45_sonnet_first_party_does_not_synthesize_structured_outputs() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-5-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-sonnet-4-5-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    let beta = calls[0].headers.get("anthropic-beta").expect("beta header");
    for token in ["claude-code-20250219", "prompt-caching-scope-2026-01-05"] {
        assert!(beta.contains(token), "missing {token} in {beta}");
    }
    assert!(
        !beta.contains("structured-outputs-2025-12-15"),
        "router should not synthesize structured-outputs by default: {beta}"
    );
}

#[tokio::test]
#[serial]
async fn test_sonnet46_first_party_includes_context_1m() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-6-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-sonnet-4-6-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    let beta = calls[0].headers.get("anthropic-beta").expect("beta header");
    assert!(
        beta.contains("context-1m-2025-08-07"),
        "Sonnet 4.6+ should receive context-1m beta: {beta}"
    );
}

/// Test 7: An Anthropic OAuth account on a 1P provider gets the
/// `oauth-2025-04-20` beta, and the metadata user_id payload carries the
/// account UUID. The Authorization header reflects the OAuth bearer token.
#[tokio::test]
#[serial]
async fn test_oauth_account_includes_oauth_beta_and_account_uuid() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let state = setup_state().await;
    let provider = create_provider(
        &state,
        "claude-oauth",
        &upstream,
        None,
        "bearer",
        Some("Bearer"),
        "claude-oauth",
        "claude-oauth/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    create_oauth_account(
        &state,
        &provider.id,
        "oauth-tok",
        "rt-7",
        Utc::now() + ChronoDuration::hours(1),
        Some("acc-uuid-7"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("claude-oauth/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];

    let beta = call.headers.get("anthropic-beta").expect("beta header");
    assert!(
        beta.contains("oauth-2025-04-20"),
        "OAuth subscriber should receive oauth beta: {beta}"
    );

    let user_id_raw = call
        .body
        .get("metadata")
        .and_then(|m| m.get("user_id"))
        .and_then(Value::as_str)
        .expect("metadata.user_id");
    let user_id: Value = serde_json::from_str(user_id_raw).expect("parse user_id");
    assert_eq!(user_id["account_uuid"].as_str(), Some("acc-uuid-7"));

    assert_eq!(
        call.headers.get("authorization").map(String::as_str),
        Some("Bearer oauth-tok")
    );
}

/// Test 8: When the OAuth token is within the 5-minute proactive-refresh
/// window, the router refreshes BEFORE sending the upstream call, then uses
/// the refreshed token in the messages request.
#[tokio::test]
#[serial]
async fn test_oauth_proactive_refresh_when_token_near_expiry() {
    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;

    let mut env = EnvGuard::new();
    env.set(
        "KOU_CC_CLAUDE_TOKEN_URL",
        &format!("{upstream}/v1/oauth/token"),
    );

    let state = setup_state().await;
    let provider = create_provider(
        &state,
        "claude-oauth",
        &upstream,
        None,
        "bearer",
        Some("Bearer"),
        "claude-oauth",
        "claude-oauth/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    create_oauth_account(
        &state,
        &provider.id,
        "old-tok",
        "old-rt",
        Utc::now() - ChronoDuration::hours(1), // expired => proactive refresh
        Some("acc-uuid-8"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("claude-oauth/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    assert_eq!(
        mock.token_call_count(),
        1,
        "proactive refresh must hit token endpoint exactly once"
    );

    let calls = mock.captured();
    assert_eq!(calls.len(), 1, "messages endpoint hit exactly once");
    assert_eq!(
        calls[0].headers.get("authorization").map(String::as_str),
        Some("Bearer refreshed-tok"),
        "messages call should carry the refreshed token, not the expired one"
    );

    drop(env);
}

/// Test 9: When the upstream returns 401 on an OAuth-backed call, the router
/// force-refreshes the token and retries the same request once.
#[tokio::test]
#[serial]
async fn test_oauth_401_triggers_refresh_and_retry() {
    let mock = MockState::new(vec![
        (
            StatusCode::UNAUTHORIZED,
            json!({"error": {"type": "authentication_error", "message": "oauth token expired"}}),
        ),
        (StatusCode::OK, default_messages_response()),
    ]);
    let upstream = spawn_mock(mock.clone()).await;

    let mut env = EnvGuard::new();
    env.set(
        "KOU_CC_CLAUDE_TOKEN_URL",
        &format!("{upstream}/v1/oauth/token"),
    );

    let state = setup_state().await;
    let provider = create_provider(
        &state,
        "claude-oauth",
        &upstream,
        None,
        "bearer",
        Some("Bearer"),
        "claude-oauth",
        "claude-oauth/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    create_oauth_account(
        &state,
        &provider.id,
        "old-tok",
        "old-rt",
        Utc::now() + ChronoDuration::hours(1), // fresh => no proactive refresh
        Some("acc-uuid-9"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("claude-oauth/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(
        calls.len(),
        2,
        "messages endpoint called twice (401 + retry)"
    );
    assert_eq!(
        mock.token_call_count(),
        1,
        "force-refresh hits token endpoint between the two messages calls"
    );
    assert_eq!(
        calls[0].headers.get("authorization").map(String::as_str),
        Some("Bearer old-tok"),
        "first attempt uses original token"
    );
    assert_eq!(
        calls[1].headers.get("authorization").map(String::as_str),
        Some("Bearer refreshed-tok"),
        "retry attempt uses the refreshed token"
    );

    drop(env);
}

/// Test 10: `KOU_CC_WORKLOAD` env propagates into the User-Agent suffix and
/// into the `cc_workload=` token in the body attribution header.
#[tokio::test]
#[serial]
async fn test_workload_env_in_ua_and_billing_header() {
    let mut env = EnvGuard::new();
    env.set("KOU_CC_WORKLOAD", "cron-task");

    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;
    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    let call = &calls[0];

    let ua = call.headers.get("user-agent").expect("user-agent");
    assert!(
        ua.contains(", workload/cron-task"),
        "UA should embed workload tag: {ua}"
    );

    let attribution = call
        .body
        .get("system")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .expect("attribution block");
    assert!(
        attribution.contains(" cc_workload=cron-task;"),
        "attribution should carry cc_workload: {attribution}"
    );

    drop(env);
}

/// Test 11: `ANTHROPIC_CUSTOM_HEADERS` (newline-separated `Name: Value`
/// pairs) appears verbatim on the upstream request, normalized to lowercase
/// header names by the HTTP layer.
#[tokio::test]
#[serial]
async fn test_custom_headers_env_passed_through() {
    let mut env = EnvGuard::new();
    env.set("ANTHROPIC_CUSTOM_HEADERS", "X-Foo: bar\nX-Baz: qux");

    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;
    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    let headers = &calls[0].headers;
    assert_eq!(headers.get("x-foo").map(String::as_str), Some("bar"));
    assert_eq!(headers.get("x-baz").map(String::as_str), Some("qux"));

    drop(env);
}

/// Test 12: `CLAUDE_CODE_ADDITIONAL_PROTECTION` truthy emits the
/// `x-anthropic-additional-protection: true` header on outgoing requests.
#[tokio::test]
#[serial]
async fn test_additional_protection_env_emits_header() {
    let mut env = EnvGuard::new();
    env.set("CLAUDE_CODE_ADDITIONAL_PROTECTION", "1");

    let mock = MockState::default_messages();
    let upstream = spawn_mock(mock.clone()).await;
    let state = setup_state().await;
    create_provider(
        &state,
        "anthropic",
        &upstream,
        Some("sk-test"),
        "x-api-key",
        None,
        "anthropic",
        "anthropic/claude-sonnet-4-20250514",
        Some("claude"),
    )
    .await;
    let app = build_app(state);

    let resp = post_messages(
        app,
        messages_request_body("anthropic/claude-sonnet-4-20250514"),
        &[],
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let calls = mock.captured();
    assert_eq!(
        calls[0]
            .headers
            .get("x-anthropic-additional-protection")
            .map(String::as_str),
        Some("true"),
    );

    drop(env);
}
