//! Локальный колбек-листенер для Codex OAuth.
//!
//! ChatGPT редиректит браузер на `http://localhost:1455/auth/callback` —
//! в оригинале этот порт слушает Codex CLI. Здесь листенер на время
//! авторизации поднимает сам роутер: ловит `code`+`state`, завершает обмен
//! через [`OAuthService::complete_authorization`] и гаснет.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::{Query, State},
    response::Html,
    routing::get,
};
use serde::Deserialize;
use tokio::sync::Notify;

use super::service::{OAuthCompleteAuthorizationRequest, OAuthService};

const CALLBACK_ADDR: &str = "127.0.0.1:1455";
/// Совпадает с TTL oauth-сессии (10 минут) — дольше ждать некого.
const LISTEN_FOR: Duration = Duration::from_secs(600);

/// True для redirect_uri, который обслуживает этот листенер.
pub fn is_local_callback(redirect_uri: &str) -> bool {
    let uri = redirect_uri.trim();
    uri.starts_with("http://localhost:1455") || uri.starts_with("http://127.0.0.1:1455")
}

/// Поднимает листенер на 1455, если порт свободен. `AddrInUse` — не ошибка:
/// там уже живёт листенер от предыдущего start (сессии ищутся по state из
/// query, так что он обслужит и новую), либо настоящий Codex CLI.
pub async fn spawn(oauth: OAuthService) {
    // в Docker loopback контейнера снаружи недостижим — KOU_OAUTH_CALLBACK_BIND=0.0.0.0:1455
    let addr =
        std::env::var("KOU_OAUTH_CALLBACK_BIND").unwrap_or_else(|_| CALLBACK_ADDR.to_string());
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => return,
        Err(err) => {
            tracing::warn!(%err, "oauth callback listener failed to bind {addr}");
            return;
        }
    };
    tracing::info!("oauth callback listener on http://{addr}/auth/callback");

    let done = Arc::new(Notify::new());
    let app = Router::new()
        .route("/auth/callback", get(handle_callback))
        .with_state((oauth, done.clone()));

    tokio::spawn(async move {
        let shutdown = async move {
            tokio::select! {
                _ = done.notified() => {},
                _ = tokio::time::sleep(LISTEN_FOR) => {},
            }
        };
        if let Err(err) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::warn!(%err, "oauth callback listener error");
        }
        tracing::info!("oauth callback listener stopped");
    });
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn handle_callback(
    State((oauth, done)): State<(OAuthService, Arc<Notify>)>,
    Query(query): Query<CallbackQuery>,
) -> Html<String> {
    let (code, state) = match (query.code, query.state) {
        (Some(code), Some(state)) => (code, state),
        _ => {
            let detail = query
                .error_description
                .or(query.error)
                .unwrap_or_else(|| "missing code or state in callback".into());
            return Html(page("AUTHORIZATION FAILED", &detail, false));
        }
    };

    match oauth
        .complete_authorization(OAuthCompleteAuthorizationRequest { state, code })
        .await
    {
        Ok(account) => {
            let who = account
                .remote_email
                .or(account.label)
                .unwrap_or_else(|| "account".into());
            // успех — листенер больше не нужен, освобождаем порт для Codex CLI
            done.notify_waiters();
            Html(page(
                "ACCOUNT CONNECTED",
                &format!("{who} linked — close this tab and return to kou-router."),
                true,
            ))
        }
        Err(err) => Html(page("AUTHORIZATION FAILED", &err.to_string(), false)),
    }
}

fn page(title: &str, detail: &str, ok: bool) -> String {
    let accent = if ok { "#2bc7a4" } else { "#ff4f30" };
    let detail = detail
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>kou-router — oauth callback</title>
<style>
  :root{{color-scheme:dark}}
  body{{margin:0;min-height:100vh;display:grid;place-items:center;
    background:#08090c;color:#e8e4da;font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace}}
  body::before{{
    content:"";position:fixed;inset:0;z-index:0;pointer-events:none;
    background:
      repeating-linear-gradient(0deg, rgba(255,255,255,.017) 0 1px, transparent 1px 32px),
      repeating-linear-gradient(90deg, rgba(255,255,255,.017) 0 1px, transparent 1px 32px);
    -webkit-mask-image:radial-gradient(ellipse 95% 90% at 50% 22%, #000 35%, transparent 80%);
    mask-image:radial-gradient(ellipse 95% 90% at 50% 22%, #000 35%, transparent 80%);
  }}
  main{{position:relative;z-index:1;width:min(420px,86vw);padding:28px 26px;text-align:center;
    border:1px solid #23272d;border-radius:10px;background:#101316;
    box-shadow:0 18px 50px rgba(0,0,0,.5)}}
  .kana{{color:#8d8678;font-size:11px;letter-spacing:.35em;margin:0 0 2px}}
  h1{{font-size:15px;letter-spacing:.18em;margin:0 0 14px;color:{accent}}}
  p{{margin:0;font-size:12.5px;color:#a1a8af;word-break:break-word}}
</style></head><body>
<main>
  <p class="kana">改札 KOU-ROUTER</p>
  <h1>{title}</h1>
  <p>{detail}</p>
</main>
</body></html>
"#
    )
}
