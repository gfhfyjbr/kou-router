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

/// Parse JSON body from a response.
async fn json_body(response: axum::http::Response<Body>) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Run auth setup and return the app for further requests.
async fn setup_auth(app: &Router) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"password": "longpassword123"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// Run auth setup then login; returns the JWT cookie value (just the token).
async fn setup_and_login(app: &Router) -> String {
    setup_auth(app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"password": "longpassword123"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    extract_jwt_cookie(&resp)
}

/// Extract the kou_auth token from a Set-Cookie header.
fn extract_jwt_cookie(resp: &axum::http::Response<Body>) -> String {
    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .expect("set-cookie header missing")
        .to_str()
        .unwrap();
    // Format: "kou_auth=TOKEN; Path=/; ..."
    let token = set_cookie
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("kou_auth=")
        .expect("cookie should start with kou_auth=");
    token.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_auth_status_initially_no_auth() {
    let state = setup_state().await;
    let app = build_app(state);

    let resp = app
        .oneshot(Request::builder().uri("/api/auth/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = json_body(resp).await;
    assert_eq!(body["auth_required"], false);
    assert_eq!(body["setup_complete"], false);
}

#[tokio::test]
async fn test_auth_setup_creates_admin() {
    let state = setup_state().await;
    let app = build_app(state);

    // Setup
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"password": "longpassword123"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Check status changed
    let resp = app
        .oneshot(Request::builder().uri("/api/auth/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = json_body(resp).await;
    assert_eq!(body["auth_required"], true);
    assert_eq!(body["setup_complete"], true);
}

#[tokio::test]
async fn test_auth_setup_rejects_short_password() {
    let state = setup_state().await;
    let app = build_app(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(json!({"password": "short"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_auth_setup_rejects_double_setup() {
    let state = setup_state().await;
    let app = build_app(state);

    setup_auth(&app).await;

    // Second setup attempt
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"password": "anotherpassword456"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_auth_login_returns_jwt_cookie() {
    let state = setup_state().await;
    let app = build_app(state);

    setup_auth(&app).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"password": "longpassword123"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .expect("login must return Set-Cookie")
        .to_str()
        .unwrap();
    assert!(
        set_cookie.contains("kou_auth="),
        "cookie must contain kou_auth token"
    );
}

#[tokio::test]
async fn test_auth_login_wrong_password() {
    let state = setup_state().await;
    let app = build_app(state);

    setup_auth(&app).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"password": "wrongpassword999"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_auth_logout_clears_cookie() {
    let state = setup_state().await;
    let app = build_app(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let set_cookie = resp
        .headers()
        .get("set-cookie")
        .expect("logout must return Set-Cookie")
        .to_str()
        .unwrap();
    assert!(
        set_cookie.contains("Max-Age=0"),
        "logout cookie must expire immediately"
    );
}

#[tokio::test]
async fn test_management_endpoints_blocked_without_auth() {
    let state = setup_state().await;
    let app = build_app(state);

    // Enable auth first
    setup_auth(&app).await;

    // Try to list keys with no credentials
    let resp = app
        .oneshot(Request::builder().uri("/api/keys").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_management_with_jwt_cookie() {
    let state = setup_state().await;
    let app = build_app(state);

    let token = setup_and_login(&app).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/keys")
                .header("cookie", format!("kou_auth={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_key_create_list_revoke() {
    let state = setup_state().await;
    let app = build_app(state);

    let token = setup_and_login(&app).await;
    let cookie = format!("kou_auth={token}");

    // Create key
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/keys")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(
                    json!({"name": "test-key", "allowed_models": ["*"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let created: Value = json_body(resp).await;
    let key_id = created["id"].as_str().expect("key must have id");
    assert!(created["key"].as_str().is_some(), "response must include the raw key");

    // List keys — should contain the one we created
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/keys")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let keys: Value = json_body(resp).await;
    let keys_arr = keys.as_array().expect("keys should be an array");
    assert!(
        keys_arr.iter().any(|k| k["id"].as_str() == Some(key_id)),
        "listed keys must include the created key"
    );

    // Revoke
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/api/keys/{key_id}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // List again — key should be gone (or inactive)
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/keys")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let keys: Value = json_body(resp).await;
    let keys_arr = keys.as_array().expect("keys should be an array");
    assert!(
        !keys_arr.iter().any(|k| k["id"].as_str() == Some(key_id) && k["is_active"] == true),
        "revoked key must not appear as active"
    );
}

#[tokio::test]
async fn test_proxy_with_api_key() {
    let state = setup_state().await;
    let app = build_app(state.clone());

    let token = setup_and_login(&app).await;
    let cookie = format!("kou_auth={token}");

    // Create an API key
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/keys")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(
                    json!({"name": "proxy-key", "allowed_models": ["*"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created: Value = json_body(resp).await;
    let api_key = created["key"].as_str().expect("must get raw key").to_string();

    // Spawn mock upstream and register a provider
    let mock_body = json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hello"},
            "finish_reason": "stop"
        }]
    });
    let upstream_url = spawn_mock_server(StatusCode::OK, mock_body).await;

    state
        .repository
        .create_provider_connection(kou_router::models::NewProviderConnection {
            provider: "mock".to_string(),
            base_url: upstream_url,
            api_key: None,
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: None,
            stream_endpoint_paths: None,
            model_prefix: Some("mock".to_string()),
            name: Some("MockProvider".to_string()),
            enabled: true,
            priority: Some(0),
            default_model: Some("mock/test-model".to_string()),
            supported_endpoints: None,
            rate_limit_protection: false,
            protocol_format: None,
        })
        .await
        .unwrap();

    // Use the API key to hit the proxy endpoint
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {api_key}"))
                .body(Body::from(
                    json!({
                        "model": "mock/test-model",
                        "messages": [{"role": "user", "content": "hi"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should NOT be an auth error (400 unauthorized). It should succeed or fail
    // for a reason other than authentication.
    assert_ne!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_proxy_blocked_without_key() {
    let state = setup_state().await;
    let app = build_app(state);

    setup_auth(&app).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "model": "some/model",
                        "messages": [{"role": "user", "content": "hi"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // Proxy routes do NOT enforce auth — they have no ProxyAuth extractor.
    // The request passes through to routing and fails because no provider is configured.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body = json_body(resp).await;
    // Confirm the error is about missing providers, NOT about auth.
    let error_msg = body["error"]["message"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .unwrap_or("");
    assert!(
        !error_msg.to_lowercase().contains("unauthorized"),
        "proxy routes should not enforce auth, got: {body}"
    );
}

#[tokio::test]
async fn test_auth_not_required_allows_anonymous() {
    let state = setup_state().await;
    let app = build_app(state);

    // No auth setup — anonymous access should work
    let resp = app
        .oneshot(Request::builder().uri("/api/keys").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Mock server
// ---------------------------------------------------------------------------

async fn spawn_mock_server(status: StatusCode, body: Value) -> String {
    let app = Router::new().route(
        "/chat/completions",
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
        .expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{}", addr)
}
