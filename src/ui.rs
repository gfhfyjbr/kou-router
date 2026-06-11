//! Отдача собранного веб-UI (`frontend/dist`).
//!
//! В release dist встраивается в бинарь на этапе компиляции (rust-embed),
//! в debug файлы читаются с диска на каждый запрос — удобно для разработки.
//! `KOU_FRONTEND_DIST` переопределяет на произвольную папку в любом профиле.
//!
//! При включённом auth SPA вообще не отдаётся без валидной JWT-куки:
//! любой не-API путь получает встроенную страницу логина (401).

use axum::{
    Router,
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

use crate::auth::middleware::{has_valid_admin_cookie, is_auth_required};
use crate::routes::AppState;

#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct Dist;

/// Вешает веб-UI fallback'ом на роутер: `/` и любой не-API путь отдают SPA,
/// хешированные ассеты получают immutable cache-заголовки.
pub fn attach(router: Router, state: AppState) -> Router {
    router.fallback(move |method: Method, uri: Uri, headers: HeaderMap| {
        serve(method, uri, headers, state.clone())
    })
}

async fn serve(method: Method, uri: Uri, headers: HeaderMap, state: AppState) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = uri.path().trim_start_matches('/');
    // несматченные API-пути остаются 404, а не SPA-шеллом
    if path == "api" || path == "v1" || path.starts_with("api/") || path.starts_with("v1/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if is_auth_required(&state).await && !has_valid_admin_cookie(&state, &headers).await {
        return login_page();
    }
    match std::env::var("KOU_FRONTEND_DIST") {
        Ok(dir) => from_disk(std::path::Path::new(&dir), path).await,
        Err(_) => from_embed(path),
    }
}

/// Самодостаточная страница логина: без ассетов, после успешного
/// `/api/auth/login` перезагружает страницу — кука уже стоит, сервер
/// отдаёт SPA.
fn login_page() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        LOGIN_PAGE,
    )
        .into_response()
}

const LOGIN_PAGE: &str = r#"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>kou-router — ticket gate</title>
<style>
  :root{color-scheme:dark}
  body{margin:0;min-height:100vh;display:grid;place-items:center;
    background:#08090c;color:#e8e4da;font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace}
  /* blueprint grid + film grain — same ambience as the app behind the gate */
  body::before{
    content:"";position:fixed;inset:0;z-index:0;pointer-events:none;
    background:
      repeating-linear-gradient(0deg, rgba(255,255,255,.017) 0 1px, transparent 1px 32px),
      repeating-linear-gradient(90deg, rgba(255,255,255,.017) 0 1px, transparent 1px 32px);
    -webkit-mask-image:radial-gradient(ellipse 95% 90% at 50% 22%, #000 35%, transparent 80%);
    mask-image:radial-gradient(ellipse 95% 90% at 50% 22%, #000 35%, transparent 80%);
  }
  body::after{
    content:"";position:fixed;inset:0;z-index:900;pointer-events:none;
    opacity:.045;mix-blend-mode:overlay;
    background-image:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='180' height='180'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.8' numOctaves='2' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  }
  form{position:relative;z-index:1;width:min(320px,86vw);padding:28px 26px;
    border:1px solid #23272d;border-radius:10px;
    background:#101316;box-shadow:0 18px 50px rgba(0,0,0,.5)}
  .kana{color:#8d8678;font-size:11px;letter-spacing:.35em;margin:0 0 2px}
  h1{font-size:15px;letter-spacing:.18em;margin:0 0 18px}
  input{width:100%;box-sizing:border-box;padding:10px 12px;margin:0 0 12px;
    background:#08090c;border:1px solid #23272d;border-radius:6px;color:inherit;font:inherit}
  input:focus{outline:none;border-color:#ff4f30}
  button{width:100%;padding:10px;border:0;border-radius:6px;cursor:pointer;
    background:#ff4f30;color:#fff;font:inherit;letter-spacing:.18em}
  button:hover{background:#ff6347}
  #err{min-height:18px;margin:10px 0 0;color:#ff4f30;font-size:12px}
  /* pointer spotlight + custom cursor — same ambience as the app */
  .spot{
    position:fixed;left:0;top:0;width:560px;height:560px;margin:-280px 0 0 -280px;
    z-index:0;pointer-events:none;opacity:0;transition:opacity .8s ease;
    background:radial-gradient(circle, rgba(255,122,64,.055), rgba(255,79,48,.022) 42%, transparent 68%);
    mix-blend-mode:screen;will-change:transform;
  }
  .no-cursor, .no-cursor *{cursor:none!important}
  .cursor-dot{
    position:fixed;left:0;top:0;z-index:950;width:6px;height:6px;margin:-3px 0 0 -3px;
    border-radius:50%;background:#ff4f30;pointer-events:none;opacity:0;
    box-shadow:0 0 10px rgba(255,79,48,.9);mix-blend-mode:screen;
    transition:width .25s cubic-bezier(.34,1.56,.64,1), height .25s cubic-bezier(.34,1.56,.64,1),
      margin .25s cubic-bezier(.34,1.56,.64,1), border-radius .25s, background .25s, box-shadow .25s;
  }
  .cursor-dot.txt{
    width:2.5px;height:21px;margin:-10.5px 0 0 -1.25px;border-radius:2px;
    background:#eeebe1;box-shadow:0 0 9px rgba(238,235,225,.55);
    animation:caretBlink 1.1s ease-in-out infinite;
  }
  @keyframes caretBlink{0%,55%{opacity:1}75%,95%{opacity:.2}100%{opacity:1}}
  .cursor-ring{
    position:fixed;left:0;top:0;z-index:950;width:30px;height:30px;margin:-15px 0 0 -15px;
    border-radius:50%;border:1.5px solid rgba(238,235,225,.35);pointer-events:none;opacity:0;
    transition:width .3s cubic-bezier(.34,1.56,.64,1), height .3s cubic-bezier(.34,1.56,.64,1),
      margin .3s cubic-bezier(.34,1.56,.64,1), border-color .3s, background .3s, box-shadow .3s, opacity .3s;
  }
  .cursor-ring.act{
    width:48px;height:48px;margin:-24px 0 0 -24px;
    border-color:rgba(255,79,48,.85);background:rgba(255,79,48,.07);
    box-shadow:0 0 22px rgba(255,79,48,.28), inset 0 0 14px rgba(255,79,48,.14);
  }
  .cursor-ring.txt{opacity:0!important}
</style></head><body>
<form id="f">
  <p class="kana">改札 KOU-ROUTER</p>
  <h1>TICKET GATE</h1>
  <input id="p" type="password" placeholder="admin password" autofocus autocomplete="current-password">
  <button type="submit">ENTER</button>
  <p id="err"></p>
</form>
<div class="spot" id="spot" aria-hidden="true"></div>
<div class="cursor-dot" id="cdot" aria-hidden="true"></div>
<div class="cursor-ring" id="cring" aria-hidden="true"></div>
<script>
document.getElementById('f').addEventListener('submit', async (ev) => {
  ev.preventDefault();
  const err = document.getElementById('err');
  err.textContent = '';
  try {
    const r = await fetch('/api/auth/login', {
      method: 'POST',
      headers: {'content-type': 'application/json'},
      body: JSON.stringify({password: document.getElementById('p').value}),
    });
    if (r.ok) { location.reload(); return; }
    err.textContent = 'invalid password';
  } catch { err.textContent = 'router unreachable'; }
});

/* custom cursor: dot tracks the pointer, ring and spotlight trail behind */
if (matchMedia('(pointer: fine)').matches &&
    !matchMedia('(prefers-reduced-motion: reduce)').matches) {
  const spot = document.getElementById('spot');
  const dot = document.getElementById('cdot');
  const ring = document.getElementById('cring');
  document.documentElement.classList.add('no-cursor');
  const m = {x: innerWidth / 2, y: innerHeight / 2};
  const rg = {x: m.x, y: m.y}, sp = {x: m.x, y: m.y};
  let shown = false, idle = true, mag = null;
  const follow = () => {
    dot.style.transform = `translate3d(${m.x}px,${m.y}px,0)`;
    rg.x += (m.x - rg.x) * 0.22; rg.y += (m.y - rg.y) * 0.22;
    ring.style.transform = `translate3d(${rg.x}px,${rg.y}px,0)`;
    sp.x += (m.x - sp.x) * 0.055; sp.y += (m.y - sp.y) * 0.055;
    spot.style.transform = `translate3d(${sp.x}px,${sp.y}px,0)`;
    if (Math.abs(m.x - rg.x) < 0.3 && Math.abs(m.y - rg.y) < 0.3 &&
        Math.abs(m.x - sp.x) < 0.3 && Math.abs(m.y - sp.y) < 0.3) { idle = true; return; }
    requestAnimationFrame(follow);
  };
  addEventListener('pointermove', (e) => {
    m.x = e.clientX; m.y = e.clientY;
    if (!shown) {
      shown = true;
      spot.style.opacity = '1'; dot.style.opacity = '1'; ring.style.opacity = '1';
    }
    const t = e.target;
    const overText = !!(t && t.closest && t.closest('input'));
    const b = t && t.closest ? t.closest('button') : null;
    ring.classList.toggle('act', !overText && !!b);
    ring.classList.toggle('txt', overText);
    dot.classList.toggle('txt', overText);
    /* magnetic button pull, same coefficients as the app */
    if (mag && mag !== b) { mag.style.transform = ''; mag = null; }
    if (b) {
      mag = b;
      const r = b.getBoundingClientRect();
      b.style.transform =
        `translate(${((e.clientX - r.left - r.width / 2) * 0.16).toFixed(1)}px,` +
        `${((e.clientY - r.top - r.height / 2) * 0.26).toFixed(1)}px)`;
    }
    if (idle) { idle = false; requestAnimationFrame(follow); }
  }, {passive: true});
}
</script></body></html>
"#;

fn from_embed(path: &str) -> Response {
    if let Some(file) = Dist::get(path) {
        return asset_response(path, file.data.into_owned());
    }
    match Dist::get("index.html") {
        Some(file) => asset_response("index.html", file.data.into_owned()),
        None => ui_missing(),
    }
}

async fn from_disk(dir: &std::path::Path, path: &str) -> Response {
    let rel = if path.is_empty() { "index.html" } else { path };
    let mut candidate = dir.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
            return StatusCode::NOT_FOUND.into_response();
        }
        candidate.push(part);
    }
    if let Ok(bytes) = tokio::fs::read(&candidate).await {
        return asset_response(rel, bytes);
    }
    match tokio::fs::read(dir.join("index.html")).await {
        Ok(bytes) => asset_response("index.html", bytes),
        Err(_) => ui_missing(),
    }
}

fn asset_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // у vite-ассетов хеш в имени — можно кешировать навсегда
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
        ],
        bytes,
    )
        .into_response()
}

fn ui_missing() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<h1>kou-router</h1><p>Web UI not built. Run <code>cd frontend && bun install && bun run build</code> \
         and rebuild, set <code>KOU_FRONTEND_DIST</code> to a built dist, \
         or use the vite dev server (<code>bun run dev</code>).</p>",
    )
        .into_response()
}
