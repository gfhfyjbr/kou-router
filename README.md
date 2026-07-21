# kou-router

<p align="center">
  <img src="docs/assets/kou-router-switchyard-hero.svg" alt="Kou Router switchyard" width="100%">
</p>

<p align="center">
  <strong>A local LLM switchyard for OpenAI, Anthropic, Responses, Gemini, Ollama, search, audio, media, and OAuth-backed accounts.</strong>
</p>

<p align="center">
  Rust 2024 | axum | SQLite | sqlx | reqwest/rustls | React/Vite admin UI | default port <code>20128</code>
</p>

`kou-router` sits between clients and upstream model providers. Clients keep one
base URL; the router resolves model prefixes and aliases, chooses a provider
account, translates protocol shape when needed, handles OAuth refresh/failover,
and returns the response in the protocol the client expected.

It is built for local operator control: many providers, many accounts per
provider, one management UI, explicit auth, no hidden global proxy behavior, and
request logs that make routing decisions inspectable.

## Design Showcase

<table>
  <tr>
    <td width="50%">
      <img src="docs/assets/showcase-overview.png" alt="Kou Router overview switchyard">
      <br>
      <sub><strong>Overview.</strong> Live switchyard, endpoint ingress, line status, synthetic traffic, and model/account counters.</sub>
    </td>
    <td width="50%">
      <img src="docs/assets/showcase-providers.png" alt="Kou Router provider lines">
      <br>
      <sub><strong>Lines.</strong> Provider connections, OAuth/API-key accounts, priorities, readiness, refresh controls, and per-account proxy state.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/assets/showcase-models.png" alt="Kou Router model aliases">
      <br>
      <sub><strong>Models.</strong> Routed model inventory and aliases for stable client-facing names.</sub>
    </td>
    <td width="50%">
      <img src="docs/assets/showcase-logs.png" alt="Kou Router request logs">
      <br>
      <sub><strong>Logs.</strong> Request history with endpoint, resolved model, upstream account, attempts, status, latency, tokens, and cost fields.</sub>
    </td>
  </tr>
</table>

## What It Does

| Area | Detail |
|---|---|
| Unified ingress | OpenAI Chat Completions, OpenAI Responses, Anthropic Messages, Ollama chat, embeddings, images, music, video, moderation, rerank, search, TTS, and STT routes under one local server. |
| Protocol translation | Request bodies, JSON responses, and SSE streams are adapted between OpenAI-style chat, Responses, Claude Messages, Gemini, and Ollama surfaces where the target provider requires it. |
| Provider switchyard | Provider connections have prefixes, priorities, default models, custom endpoint paths, supported endpoint sets, and optional protocol-format hints. |
| Account routing | Each provider can hold multiple API-key or OAuth accounts with priority, enable/disable state, last-use tracking, rate-limit state, circuit opening, backoff, and refresh errors. |
| OAuth control | Claude/Anthropic and Codex/ChatGPT-style accounts use PKCE, persist refresh tokens, refresh before expiry, and retry once after an upstream 401. |
| Operator UI | The embedded React UI manages provider lines, account sessions, API keys, model aliases, settings, rate-limit state, and request logs. |
| Local auth | Management routes use the admin JWT cookie; proxy routes can require generated API keys with optional model allowlists. |
| Observability | Request logs preserve the requested model, resolved model, account, endpoint, attempts, status, timings, token counts, cache reads, and extracted cost. |

## Routing Model

1. A client calls a local route such as `/v1/chat/completions`,
   `/v1/messages`, `/v1/responses`, or `/v1/api/chat`.
2. The router normalizes the payload and resolves the requested model through
   aliases, prefixes, provider defaults, and combo definitions.
3. It selects an enabled account, skipping rate-limited or circuit-open
   accounts. Priority routing is the default; round-robin is available for
   account and combo strategies.
4. The body and headers are adapted to the upstream protocol. Streaming
   responses are translated back as SSE when the client route expects SSE.
5. The request is retried according to routing/backoff rules, token refresh is
   attempted when needed, and the final result is logged.

The result is a single client configuration that can fan out across first-party
OAuth accounts, ordinary API-key providers, search APIs, local Ollama-style
servers, and specialty media endpoints.

## Protocol Surface

| Client route | Surface |
|---|---|
| `GET /v1`, `GET /v1/models` | OpenAI-compatible model list across enabled providers. |
| `POST /v1/chat/completions` | OpenAI Chat Completions ingress; can route to OpenAI-style, Claude Messages, Responses, Gemini, or Ollama-backed providers. |
| `POST /v1/completions` | Legacy completions ingress normalized into the chat-family router. |
| `POST /v1/messages` | Anthropic Messages ingress, including translation to other chat-family targets when configured. |
| `POST /v1/messages/count_tokens` | Raw Anthropic count-tokens passthrough. |
| `GET /v1/files/{file_id}/content` | Anthropic file-content passthrough for providers that expose it. |
| `POST /v1/responses`, `POST /v1/responses/{*path}` | OpenAI Responses ingress and Responses subroutes. |
| `POST /v1/api/chat` | Ollama chat ingress. |
| `GET/POST /v1/embeddings` | Embedding model list and embedding requests. |
| `GET/POST /v1/images/generations` | Image model list and image generation requests. |
| `GET/POST /v1/music/generations` | Music model list and music generation requests. |
| `GET/POST /v1/videos/generations` | Video model list and video generation requests. |
| `POST /v1/moderations` | Moderation requests. |
| `POST /v1/rerank` | Rerank requests. |
| `GET/POST /v1/search` | Search model list and web-search requests. |
| `POST /v1/audio/speech` | Text-to-speech; returns binary audio. |
| `POST /v1/audio/transcriptions` | Speech-to-text multipart uploads. |

## Provider Presets

Built-in presets are meant to make the first provider line fast to create while
keeping every connection explicit in the database.

| Family | Presets |
|---|---|
| API-key LLM providers | `openai`, `anthropic`, `openrouter`, `deepseek`, `groq`, `xai`, `mistral`, `together`, `fireworks`, `cohere`, `nvidia`, `nebius`, `hyperbolic`, `huggingface`, `vertex`, `alibaba`, `cloudflare-ai`, `aimlapi`, `pollinations`, `glm`, `kimi` |
| Search providers | `serper-search`, `brave-search`, `exa-search`, `tavily-search`, `perplexity-search` |
| OAuth providers | `claude-oauth`, `antigravity`, `codex`, `github-copilot` |

Presets store the base URL, auth header style, endpoint mappings, supported
endpoint families, default model, and any required static headers. Importing a
preset creates a normal provider connection that can be edited or deleted.

## OAuth Accounts

OAuth accounts are first-class provider accounts. They can be prioritized,
disabled, refreshed manually, routed through a per-account proxy, and retried
after refresh just like API-key accounts.

| Provider family | Behavior |
|---|---|
| Claude/Anthropic | PKCE authorize flow, profile enrichment, role check, Claude CLI API-key creation when inference scope is absent, refresh scope narrowing, and automatic `oauth-2025-04-20` beta/header handling for first-party OAuth inference. |
| Codex/ChatGPT | PKCE authorize flow, upstream-compatible authorize URL shape, refresh-token flow, id-token exchange for a Codex API key, account metadata extraction, and FedRAMP header propagation when the account claims it. |

Refresh policy is intentionally simple:

1. Refresh before inference when the access token expires in less than five
   minutes.
2. If upstream returns 401, force one refresh and retry once.
3. Preserve the stored API key on refresh unless the first authorization flow
   minted a new one.

## Per-Account Proxy

Each provider account can use its own HTTP, HTTPS, SOCKS5, or SOCKS5h proxy.
That proxy is used for inference, OAuth token refresh, and the first token
exchange. Global `HTTP_PROXY` and `HTTPS_PROXY` are not used for provider
traffic.

```bash
curl -sX POST http://127.0.0.1:20128/api/provider-accounts/<id>/proxy \
  -H 'content-type: application/json' \
  -b 'kou_auth=<JWT>' \
  -d '{"proxy_url": "socks5h://user:pass@host:1080"}'
```

Clear it with `{"proxy_url": null}` or an empty string.

## Quick Start

Prerequisites:

- Rust toolchain
- Bun for the default frontend build path

Run the full server:

```bash
cargo run --release -- serve
```

On first run, `kou-router` prompts for the admin password in the terminal,
creates the SQLite database automatically, builds the React UI, embeds it into
the binary, and listens on `0.0.0.0:20128`.

Then open:

```text
http://127.0.0.1:20128
```

Build once and run the release binary:

```bash
cargo build --release
./target/release/kou-router serve
```

Point OpenAI-compatible clients at the local gateway:

```bash
export OPENAI_BASE_URL=http://127.0.0.1:20128/v1
export ANTHROPIC_BASE_URL=http://127.0.0.1:20128
```

## Run Modes

```bash
kou-router serve             # web UI + API, binds 0.0.0.0:20128 by default
kou-router daemon            # API-only, binds 127.0.0.1:20128 by default
kou-router serve --bind 0.0.0.0:8080 --db sqlite:///var/lib/kou/kou.db
```

| Mode | Behavior |
|---|---|
| `serve` | Full web UI plus API. Non-API paths fall back to the SPA. |
| `daemon` | API-only mode for local agent tools. `GET /` returns service-info JSON for discovery. |

The default database URL is `sqlite://kou-router.db`.

## Frontend Development

Cargo builds embed the frontend automatically. If `frontend/node_modules` is
missing, `build.rs` runs the configured package manager install first, then
builds `frontend/dist` with Vite and embeds those files with `rust-embed`.

For UI work, run the backend and Vite separately:

```bash
cargo run -- serve
cd frontend
bun install
bun run dev
```

The Vite dev server proxies `/api`, `/v1`, and `/health` to
`http://127.0.0.1:20128` by default. Override the proxy target with
`KOU_BACKEND` when needed.

If the backend is unavailable, the UI falls back to a seeded demo mode. That is
useful for visual checks and screenshots without touching real provider tokens.

The React design system lives in the separate public
[`kou-design-system`](https://github.com/gfhfyjbr/kou-design-system) repository
and is consumed as the `@kou/ui-kit` Git dependency. See
[docs/kou-design-system-git-workflow.md](docs/kou-design-system-git-workflow.md)
for clone, update, and local UI-kit development workflow.

## Management API

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Basic health JSON. |
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
| `GET` | `/api/combos` | List model combos. |
| `POST` | `/api/combos` | Create combo. |
| `GET` | `/api/models/alias` | List model aliases. |
| `POST` | `/api/models/alias` | Upsert alias. |
| `GET` | `/api/settings` | Read settings. |
| `POST` | `/api/settings` | Save settings. |
| `GET` | `/api/ratelimits` | In-memory rate-limit snapshot. |
| `GET` | `/api/logs` | List request logs. |
| `GET` | `/api/logs/{id}` | Request log detail, including upstream attempts. |

## Auth API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/auth/status` | Returns `{ auth_required, setup_complete }`. |
| `POST` | `/api/auth/setup` | First-time setup for embedded/library use. |
| `POST` | `/api/auth/login` | Sets the `kou_auth` JWT cookie. |
| `POST` | `/api/auth/logout` | Clears the cookie. |
| `GET` | `/api/keys` | List API keys without secrets. |
| `POST` | `/api/keys` | Create API key; returns the secret once. |
| `DELETE` | `/api/keys/{id}` | Revoke API key. |

Proxy auth behavior:

- If auth is disabled, proxy routes are anonymous.
- If auth is enabled, pass `Authorization: Bearer <api_key>` or
  `x-api-key: <api_key>` to proxy routes.
- Management routes use the admin login cookie.

## Docker

```bash
docker build -t kou-router .
docker run -d -p 20128:20128 \
  -e KOU_ROUTER_ADMIN_PASSWORD='your-password-here' \
  -v kou-data:/data \
  kou-router
```

Or with Compose:

```bash
KOU_ROUTER_ADMIN_PASSWORD='your-password-here' docker compose up -d
```

The Docker image builds and embeds the frontend. Runtime data lives in `/data`
and uses `sqlite:///data/kou-router.db`.

When `KOU_ROUTER_ADMIN_PASSWORD` is set, every startup updates the stored admin
password to that value. This is deliberate for non-interactive deployments.

## Configuration Reference

Core variables:

| Variable | Default | Description |
|---|---:|---|
| `KOU_ROUTER_BIND` | `0.0.0.0:20128` in `serve`, `127.0.0.1:20128` in `daemon` | Bind address. CLI `--bind` wins. |
| `KOU_ROUTER_DATABASE_URL` | `sqlite://kou-router.db` | SQLite DSN. CLI `--db` wins. |
| `KOU_ROUTER_ADMIN_PASSWORD` | unset | Non-interactive admin password bootstrap/update. |
| `KOU_FRONTEND_DIST` | unset | Runtime UI asset override instead of embedded assets. |
| `RUST_LOG` | `kou_router=info,tower_http=info` | tracing filter. |

Frontend build variables:

| Variable | Default | Description |
|---|---:|---|
| `KOU_FRONTEND_PACKAGE_MANAGER` | `bun` | Package manager used by `build.rs`; supports `bun`, `npm`, `pnpm`, and `yarn`. |
| `KOU_SKIP_FRONTEND_BUILD` | unset | Truthy value skips the frontend build and only ensures `frontend/dist` exists. Useful for backend-only experiments. |
| `KOU_BACKEND` | `http://127.0.0.1:20128` | Vite dev-server proxy target. |

Native model discovery variables:

| Variable | Default | Description |
|---|---:|---|
| `KOU_CODEX_CLIENT_VERSION` | `0.55.0` | Codex native model-list query parameter for `GET {base}/models?client_version=...`. |

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
| `CLAUDE_CODE_DISABLE_1M_CONTEXT` / `KOU_CC_DISABLE_1M_CONTEXT` | unset | Truthy value disables automatic `context-1m` beta selection. |
| `CLAUDE_CODE_ADDITIONAL_PROTECTION` / `KOU_CC_ADDITIONAL_PROTECTION` | unset | Truthy value sends `x-anthropic-additional-protection: true`. |
| `CLAUDE_CODE_CONTAINER_ID` / `KOU_CC_REMOTE_CONTAINER_ID` | unset | Sends `x-claude-remote-container-id`. |
| `CLAUDE_CODE_REMOTE_SESSION_ID` / `KOU_CC_REMOTE_SESSION_ID` | unset | Sends `x-claude-remote-session-id`. |
| `CLAUDE_AGENT_SDK_CLIENT_APP` / `CLAUDE_CODE_CLIENT_APP` / `KOU_CC_CLIENT_APP` | unset | Client app marker for UA and `x-client-app`. |
| `CLAUDE_CODE_AGENT_ID` / `KOU_CC_AGENT_ID` | unset | Sends `x-claude-code-agent-id`. |
| `CLAUDE_CODE_PARENT_AGENT_ID` / `KOU_CC_PARENT_AGENT_ID` | unset | Sends `x-claude-code-parent-agent-id`. |
| `ANTHROPIC_CUSTOM_HEADERS` / `KOU_CC_CUSTOM_HEADERS` | unset | Newline-separated `Name: Value` headers added to Anthropic requests without overwriting client headers. |

When `kou-router` synthesizes Claude Code headers, it keeps client-supplied
`anthropic-beta` unchanged and only fills missing Claude Code attribution
headers. The conservative beta set includes `claude-code`,
`interleaved-thinking`, `effort`, `prompt-caching-scope`, explicit 1M-context
cases, Anthropic OAuth beta, and safe 3P/Vertex betas where applicable.

OAuth test overrides:

| Variable | Description |
|---|---|
| `KOU_CC_CODEX_ISSUER` | Test issuer for Codex authorize/token/api-key URL construction. |
| `CODEX_REFRESH_TOKEN_URL_OVERRIDE` | Upstream-compatible Codex refresh endpoint override. |
| `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` | Upstream-compatible Codex originator override. |
| `KOU_CC_CLAUDE_AUTHORIZE_URL` | Claude OAuth authorize URL override. |
| `KOU_CC_CLAUDE_TOKEN_URL` | Claude OAuth token URL override. |
| `KOU_CC_CLAUDE_PROFILE_URL` | Claude OAuth profile URL override. |
| `KOU_CC_CLAUDE_API_KEY_URL` | Claude OAuth API-key URL override. |
| `KOU_CC_CLAUDE_ROLES_URL` | Claude OAuth roles URL override. |

## Tests

```bash
cargo test
```

Useful focused suites:

```bash
cargo test --test fingerprint_integration
cargo test --lib test_beta_headers
cargo test --lib test_generate_headers
cargo test oauth
```

For frontend checks:

```bash
cd frontend
bun run build
```

## Project Layout

```text
src/
  main.rs             CLI entrypoint and server bootstrap
  app.rs              app builders: UI+API and headless daemon
  ui.rs               embedded web UI, SPA fallback, login gate
  routes.rs           HTTP handlers and route table
  service.rs          routing, translation, retry, fallback
  upstream.rs         upstream HTTP client and header handling
  repository.rs       SQLite repository
  db.rs               schema initialization and migrations
  models.rs           domain types and endpoint families
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
  dist/               generated at build time, embedded by Rust
docs/assets/          README hero and UI showcase images
build.rs              frontend build and embed pipeline
```

## Local Files and Secrets

Runtime databases (`*.db`), `/data`, `.vix`, downloaded upstream binaries, and
root-level visual scratch artifacts are ignored. Do not commit local SQLite
databases: they may contain provider tokens, API-key hashes, OAuth refresh
tokens, or request logs.

## License

No license has been declared yet.
