use axum::{Json, Router, routing::get};
use serde_json::json;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::routes::{AppState, router};
use crate::ui;

/// Полное приложение: API + встроенный веб-UI (режим `kou-router serve`).
pub fn build_app(state: AppState) -> Router {
    with_layers(ui::attach(router(state.clone()), state))
}

/// Headless-приложение для `kou-router daemon`: только API,
/// `/` отдаёт service-info JSON для discovery агентовыми тулами.
pub fn build_headless_app(state: AppState) -> Router {
    with_layers(router(state).route("/", get(service_info)))
}

fn with_layers(router: Router) -> Router {
    router.layer(TraceLayer::new_for_http()).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_headers(Any)
            .allow_methods(Any),
    )
}

async fn service_info() -> Json<serde_json::Value> {
    Json(json!({
        "service": "kou-router",
        "version": env!("CARGO_PKG_VERSION"),
        "mode": "daemon",
        "ui": false,
        "endpoints": {
            "health": "/health",
            "models": "/v1/models",
            "openai": "/v1/chat/completions",
            "anthropic": "/v1/messages",
            "management": "/api",
        },
    }))
}
