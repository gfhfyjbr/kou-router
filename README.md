# kou-router

`kou-router` is a Rust + axum LLM gateway. It accepts OpenAI-compatible,
Anthropic Messages, OpenAI Responses, Gemini, and Ollama-style requests,
translates between protocols, and routes them to configured upstream providers
with retries, account-level failover, rate-limit handling, and cost tracking.

- Language: Rust 2024
- HTTP server: axum 0.8 + tower-http
- Storage: SQLite through sqlx
- Upstream client: reqwest + rustls
- Default port: `0.0.0.0:20128`

## Features

- Universal proxy for OpenAI Chat Completions, OpenAI Responses, Anthropic
  Messages, Gemini, Ollama, embeddings, images, music, video, moderation,
  rerank, search, TTS, and STT endpoints.
- Protocol translation for request bodies, responses, and SSE streams.
- Multiple accounts per provider connection, with priorities, enable/disable,
  account-level proxy settings, backoff, and circuit breaking.
- OAuth support for Claude/Anthropic and Codex/ChatGPT-style accounts through
  PKCE, including proactive token refresh and one retry after upstream 401.
- Built-in provider presets for common API-key, OAuth, and search providers.
- Web admin UI, embedded into the Rust binary at compile time.
- Optional Claude Code-compatible request fingerprinting for Anthropic routes.
- API-key and JWT-cookie auth for proxy and management routes.

## Quick Start

Prerequisites:

- Rust toolchain
- Bun, by default, for building the web UI

The Rust build script builds the frontend automatically before embedding it.
If `frontend/node_modules` is missing, it runs `bun install --frozen-lockfile`;
then it runs `bun run build` and embeds `frontend/dist` with `rust-embed`.

```bash
cargo run --release -- serve
# First run: set the admin password for the web UI.
# kou-router listening (web UI + API) on 0.0.0.0:20128
```

Equivalent:

```bash
cargo build --release
./target/release/kou-router serve
```

On first run, `kou-router` asks for an admin password in the terminal and
enables auth immediately. In non-interactive environments such as Docker or
systemd, set `KOU_ROUTER_ADMIN_PASSWORD`; without a TTY and without that
variable, startup fails.

The SQLite database is created automatically. The default DSN is
`sqlite://kou-router.db`.

## Frontend Build

Frontend embedding is automatic during Cargo builds.

Environment knobs:

| Variable | Default | Description |
|---|---:|---|
| `KOU_FRONTEND_PACKAGE_MANAGER` | `bun` | Package manager used by `build.rs`; supports `bun`, `npm`, `pnpm`, `yarn`. |
| `KOU_SKIP_FRONTEND_BUILD` | unset | Truthy value skips the frontend build and only ensures `frontend/dist` exists. Useful for quick backend-only experiments. |
| `KOU_FRONTEND_DIST` | unset | Runtime override: serve the UI from this directory instead of embedded assets. |

For UI development, run Vite directly and keep the backend running:

```bash
cargo run -- serve
cd frontend
bun install
bun run dev
```

The Vite dev server proxies `/api`, `/v1`, and `/health` to
`http://127.0.0.1:20128` by default. Override with `KOU_BACKEND`.

## Run Modes

```bash
kou-router [serve]            # web UI + API, binds 0.0.0.0:20128 by default
kou-router daemon             # headless API, binds 127.0.0.1:20128 by default
kou-router serve --bind 0.0.0.0:8080 --db sqlite:///var/lib/kou/kou.db
```

- `serve`: full web UI plus API. Non-API paths fall back to the SPA.
- `daemon`: API-only mode for local agent tools. `GET /` returns service-info
  JSON for discovery.

Agent/client setup examples:

```bash
export OPENAI_BASE_URL=http://127.0.0.1:20128/v1
export ANTHROPIC_BASE_URL=http://127.0.0.1:20128
```

## Docker

```bash
docker build -t kou-router .
docker run -d -p 20128:20128 \
  -e KOU_ROUTER_ADMIN_PASSWORD='your-password-here' \
  -v kou-data:/data \
  kou-router
```

Or:

```bash
KOU_ROUTER_ADMIN_PASSWORD='your-password-here' docker compose up -d
```

The Docker image builds the frontend and embeds it into the release binary.
Runtime data lives in `/data` and uses `sqlite:///data/kou-router.db`.

`KOU_ROUTER_ADMIN_PASSWORD` is authoritative: each startup with this variable
set updates the stored admin password to that value.

## Environment

Core variables:

| Variable | Default | Description |
|---|---:|---|
| `KOU_ROUTER_BIND` | `0.0.0.0:20128` in `serve`, `127.0.0.1:20128` in `daemon` | Bind address. CLI `--bind` wins. |
| `KOU_ROUTER_DATABASE_URL` | `sqlite://kou-router.db` | SQLite DSN. CLI `--db` wins. |
| `KOU_ROUTER_ADMIN_PASSWORD` | unset | Non-interactive admin password bootstrap/update. |
| `KOU_FRONTEND_DIST` | unset | Runtime UI asset override. |
| `RUST_LOG` | `kou_router=info,tower_http=info` | tracing filter. |

Claude Code fingerprint variables:

| Variable | Default | Description |
|---|---:|---|
| `KOU_CC_FINGERPRINT` | `1` | `0` or `false` disables fingerprint injection. |
| `KOU_CC_VERSION` / `CLAUDE_CODE_VERSION` | `2.1.173` | Claude Code CLI version used in UA and billing attribution. |
| `CLAUDE_CODE_ENTRYPOINT` / `KOU_CC_ENTRYPOINT` | `cli` | Entry point marker for UA and billing attribution. |
| `KOU_CC_USER_TYPE` | `external` | User type marker. |
| `KOU_CC_WORKLOAD` | unset | Optional workload marker. |
| `CLAUDE_AGENT_SDK_VERSION` / `KOU_CC_AGENT_SDK_VERSION` | unset | Agent SDK version marker for the UA. |
| `KOU_CC_DEVICE_ID` | auto | Override stable 64-hex device id. |
| `KOU_CC_ANT_INTERNAL` | `0` | Enables `cli-internal-2026-02-09`. |
| `CLAUDE_CODE_ADDITIONAL_PROTECTION` / `KOU_CC_ADDITIONAL_PROTECTION` | unset | Truthy value sends `x-anthropic-additional-protection: true`. |
| `CLAUDE_CODE_CONTAINER_ID` / `KOU_CC_REMOTE_CONTAINER_ID` | unset | Sends `x-claude-remote-container-id`. |
| `CLAUDE_CODE_REMOTE_SESSION_ID` / `KOU_CC_REMOTE_SESSION_ID` | unset | Sends `x-claude-remote-session-id`. |
| `CLAUDE_AGENT_SDK_CLIENT_APP` / `CLAUDE_CODE_CLIENT_APP` / `KOU_CC_CLIENT_APP` | unset | Client app marker for UA and `x-client-app`. |
| `CLAUDE_CODE_AGENT_ID` / `KOU_CC_AGENT_ID` | unset | Sends `x-claude-code-agent-id`. |
| `CLAUDE_CODE_PARENT_AGENT_ID` / `KOU_CC_PARENT_AGENT_ID` | unset | Sends `x-claude-code-parent-agent-id`. |
| `ANTHROPIC_CUSTOM_HEADERS` / `KOU_CC_CUSTOM_HEADERS` | unset | Newline-separated `Name: Value` headers added to Anthropic requests without overwriting client headers. |
| `KOU_CC_CLAUDE_TOKEN_URL` | `https://platform.claude.com/v1/oauth/token` | Anthropic OAuth token endpoint override for tests. |

When `kou-router` has to synthesize Claude Code headers, it uses a conservative
`anthropic-beta` set: `claude-code`, `interleaved-thinking`, `effort`,
`prompt-caching-scope`, `context-1m` only for explicit `[1m]` or Sonnet/Opus
4.6+, OAuth beta for Anthropic OAuth accounts, and safe 3P/Vertex betas where
applicable. If the client already supplied `anthropic-beta`, the router keeps
that value unchanged and only fills in missing Claude Code headers.

## OAuth and Token Refresh

Claude/Anthropic and Codex/ChatGPT OAuth providers use PKCE. OAuth accounts can
be attached to a provider connection and used for inference just like API-key
accounts.

Refresh behavior:

1. Proactive refresh before inference if `expires_at` is less than five minutes
   away.
2. Forced refresh and one retry if upstream returns 401.

For Anthropic first-party OAuth accounts, the `oauth-2025-04-20` beta is added
automatically and `metadata.user_id.account_uuid` is filled from the remote
account id extracted during OAuth.

## HTTP API

All JSON proxy routes accept a `model` field, select provider/account routing,
and then proxy to the upstream. Streaming is enabled with `"stream": true`.

Health:

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Basic health JSON. |

Model lists:

| Method | Path | Description |
|---|---|---|
| `GET` | `/v1` | Alias for `/v1/models`. |
| `GET` | `/v1/models` | All enabled provider models. |
| `GET` | `/v1/embeddings` | Embedding-capable models. |
| `GET` | `/v1/images/generations` | Image-capable models. |
| `GET` | `/v1/music/generations` | Music-capable models. |
| `GET` | `/v1/videos/generations` | Video-capable models. |
| `GET` | `/v1/search` | Search providers/models. |

Proxy routes:

| Method | Path | Endpoint |
|---|---|---|
| `POST` | `/v1/chat/completions` | OpenAI Chat Completions |
| `POST` | `/v1/completions` | OpenAI legacy completions |
| `POST` | `/v1/messages` | Anthropic Messages |
| `POST` | `/v1/messages/count_tokens` | Raw Anthropic count_tokens proxy |
| `POST` | `/v1/responses` | OpenAI Responses |
| `POST` | `/v1/responses/{*path}` | OpenAI Responses subroutes |
| `POST` | `/v1/api/chat` | Ollama chat |
| `POST` | `/v1/embeddings` | Embeddings |
| `POST` | `/v1/images/generations` | Image generation |
| `POST` | `/v1/music/generations` | Music generation |
| `POST` | `/v1/videos/generations` | Video generation |
| `POST` | `/v1/moderations` | Moderation |
| `POST` | `/v1/rerank` | Rerank |
| `POST` | `/v1/search` | Web search |
| `POST` | `/v1/audio/speech` | TTS, returns binary audio |
| `POST` | `/v1/audio/transcriptions` | STT, multipart form-data |

Proxy auth:

- If auth is disabled, proxy routes are anonymous.
- If auth is enabled, pass `Authorization: Bearer <api_key>` or
  `x-api-key: <api_key>`.

Management routes:

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/providers` | List provider connections. |
| `POST` | `/api/providers` | Create provider connection. |
| `GET` | `/api/providers/presets` | List built-in presets. |
| `POST` | `/api/providers/import` | Import a preset. |
| `GET` | `/api/provider-accounts?provider_connection_id=...` | List accounts. |
| `POST` | `/api/provider-accounts` | Create API-key or OAuth account stub. |
| `POST` | `/api/provider-accounts/oauth/start` | Start OAuth. |
| `POST` | `/api/provider-accounts/oauth/callback` | Finish OAuth. |
| `POST` | `/api/provider-accounts/{id}/refresh` | Force token refresh. |
| `POST` | `/api/provider-accounts/{id}/proxy` | Set or clear per-account proxy. |
| `POST` | `/api/provider-accounts/{id}/enable` | Enable account. |
| `POST` | `/api/provider-accounts/{id}/disable` | Disable account. |
| `DELETE` | `/api/provider-accounts/{id}` | Delete account. |
| `GET` | `/api/combos` | List combos. |
| `POST` | `/api/combos` | Create combo. |
| `GET` | `/api/models/alias` | List model aliases. |
| `POST` | `/api/models/alias` | Upsert alias. |
| `GET` | `/api/settings` | Read settings. |
| `POST` | `/api/settings` | Save settings. |
| `GET` | `/api/ratelimits` | In-memory rate-limit snapshot. |
| `GET` | `/api/logs` | List request logs. |
| `GET` | `/api/logs/{id}` | Request log detail. |

Auth routes:

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/auth/status` | `{ auth_required, setup_complete }`. |
| `POST` | `/api/auth/setup` | First-time setup for embedded/library use. |
| `POST` | `/api/auth/login` | Sets `kou_auth` JWT cookie. |
| `POST` | `/api/auth/logout` | Clears the cookie. |
| `GET` | `/api/keys` | List API keys without secrets. |
| `POST` | `/api/keys` | Create API key; returns the secret once. |
| `DELETE` | `/api/keys/{id}` | Revoke API key. |

## Provider Presets

API-key style:

`openai`, `anthropic`, `openrouter`, `deepseek`, `groq`, `xai`, `mistral`,
`together`, `fireworks`, `cohere`, `nvidia`, `nebius`, `hyperbolic`,
`huggingface`, `vertex`, `alibaba`, `cloudflare-ai`, `aimlapi`,
`pollinations`, `glm`, `kimi`

Search:

`serper-search`, `brave-search`, `exa-search`, `tavily-search`,
`perplexity-search`

OAuth:

`claude-oauth`, `antigravity`, `codex`, `github-copilot`

## Per-Account Proxy

Each provider account can use its own HTTP, HTTPS, SOCKS5, or SOCKS5h proxy.
This proxy is used for inference, token refresh, and first token exchange.
Standard `HTTP_PROXY` / `HTTPS_PROXY` environment variables are intentionally
not used for provider traffic.

```bash
curl -sX POST http://127.0.0.1:20128/api/provider-accounts/<id>/proxy \
  -H 'content-type: application/json' \
  -b 'kou_auth=<JWT>' \
  -d '{"proxy_url": "socks5h://user:pass@host:1080"}'
```

Clear it with `{"proxy_url": null}` or an empty string.

## Tests

```bash
cargo test
```

Useful focused suites:

```bash
cargo test --test fingerprint_integration
cargo test --lib test_beta_headers
cargo test --lib test_generate_headers
```

## Project Layout

```text
src/
  main.rs             CLI entrypoint and server bootstrap
  app.rs              app builders: UI+API and headless daemon
  ui.rs               embedded web UI, SPA fallback, login gate
  routes.rs           HTTP handlers
  service.rs          routing, translation, retry, fallback
  upstream.rs         upstream HTTP client and header handling
  repository.rs       SQLite repository
  db.rs               schema initialization and migrations
  models.rs           domain types and EndpointKind
  presets.rs          built-in provider presets
  auth/               admin JWT, API keys, auth extractors
  oauth/              PKCE sessions and provider-specific OAuth
  translate/          protocol translators and SSE adapters
  fingerprint.rs      Claude Code-compatible header/body attribution
  search.rs           search provider adapters
  audio.rs            TTS/STT routing
  cost.rs             usage and cost extraction
  ratelimit.rs        rate-limit parsing and in-memory tracker
  retry.rs            retry and backoff
frontend/
  src/                React/Vite admin UI
  dist/               generated at build time, not committed
build.rs              auto-builds frontend and prepares embedded assets
```

## Local Files and Secrets

Runtime databases (`*.db`), `/data`, `.vix`, downloaded upstream binaries, and
root-level visual scratch artifacts are ignored. Do not commit local SQLite
databases: they may contain provider tokens, API-key hashes, OAuth refresh
tokens, or request logs.

## License

No license has been declared yet.
