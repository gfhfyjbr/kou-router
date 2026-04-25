# kou-router

LLM/AI-роутер на Rust + axum. Принимает запросы в формате OpenAI / Anthropic /
Gemini / Ollama, переводит их между протоколами и проксирует в любой из
поддерживаемых апстримов с балансировкой по нескольким аккаунтам, ретраями,
учётом rate-limit'ов и стоимости.

- **Язык:** Rust 2024
- **HTTP:** axum 0.8 + tower-http (CORS, tracing)
- **Хранилище:** SQLite (sqlx, runtime-tokio-rustls)
- **HTTP клиент апстрима:** reqwest + rustls
- **Порт по умолчанию:** `0.0.0.0:20128`

---

## Что умеет

- **Универсальный прокси.** Один и тот же запрос можно отправить в формате
  OpenAI Chat Completions / Responses / Anthropic Messages / Gemini / Ollama —
  роутер сам подберёт апстрим по `model`, переведёт payload и стрим в нужный
  формат и приведёт ответ обратно к формату клиента.
- **Несколько аккаунтов на провайдер.** На один `provider_connection` можно
  привязать N `provider_account` (API-ключ или OAuth), с приоритетами,
  включением/выключением, circuit-breaker'ом и backoff.
- **OAuth.** Поддержаны Claude (Anthropic Console) и Codex (ChatGPT/Codex)
  через PKCE-флоу, с автоматическим refresh.
- **Rate-limit & retry.** Парсинг апстрим-хедеров (`x-ratelimit-*`,
  `Retry-After`), внутренний tracker, экспоненциальный backoff с jitter.
- **Стоимость.** Извлечение `usage` из ответа и расчёт по таблице цен.
- **Claude Code fingerprint.** Опциональная подмена заголовков под
  Claude Code CLI для запросов в Anthropic.
- **Аудио.** `audio.speech` и multipart `audio.transcriptions`.
- **Мультимодальность.** Images / Music / Video generation, embeddings,
  moderations, rerank, search.
- **Auth.** JWT-cookie для админки + bearer/`x-api-key` для клиентских
  запросов. Скоупинг ключей по моделям.

---

## Быстрый старт

```bash
# сборка
cargo build --release

# запуск
cargo run --release
# → kou-router listening on 0.0.0.0:20128
```

База создаётся автоматически при первом запуске
(`sqlite://kou-router.db` по умолчанию).

### Переменные окружения

**Базовые:**

| Переменная | По умолчанию | Назначение |
|---|---|---|
| `KOU_ROUTER_BIND` | `0.0.0.0:20128` | host:port, на котором слушает axum |
| `KOU_ROUTER_DATABASE_URL` | `sqlite://kou-router.db` | SQLite DSN |
| `RUST_LOG` / `EnvFilter` | `kou_router=info,tower_http=info` | tracing-фильтр |

**Claude Code fingerprint** (`src/fingerprint.rs`).

При проксировании в Anthropic роутер по умолчанию маскирует запрос под
официальный Claude Code CLI: подсовывает `User-Agent: claude-cli/...`,
заголовок `x-anthropic-billing-header`, `x-claude-code-session-id`, набор
`anthropic-beta` фич (prompt-caching-scope, fast-mode, context-1m,
structured-outputs и т.д.), и инжектит `metadata.user_id` в body. Это нужно
чтобы (а) попасть в Claude-Code billing-cohort, (б) получить доступ к
1P-only beta-флагам, недоступным обычному API.

⚠️ Подмена нарушает Anthropic ToS. На свой страх и риск.

| Переменная | По умолчанию | Назначение |
|---|---|---|
| `KOU_CC_FINGERPRINT` | `1` (включено) | `0` / `false` — выключить всю подмену |
| `KOU_CC_VERSION` | `2.2.0` | Версия Claude Code CLI, под которую косим |
| `KOU_CC_ENTRYPOINT` | `cli` | Метка в `cc_entrypoint` billing-хедера и UA (`cli`/`sdk`/`vscode`/...) |
| `KOU_CC_USER_TYPE` | `external` | `external` / `internal` (Anthropic employees) |
| `KOU_CC_WORKLOAD` | — | Тег для billing attribution (e.g. `cron-task`) |
| `KOU_CC_AGENT_SDK_VERSION` | — | Версия Agent SDK, добавляется в UA |
| `KOU_CC_CLIENT_APP` | — | Свой `client-app/...` маркер в UA |
| `KOU_CC_DEVICE_ID` | auto | Переопределить device_id (ровно 64 hex chars). По умолчанию генерится один раз и кешируется в `~/.config/kou-router/device_id` |
| `KOU_CC_ANT_INTERNAL` | `0` | Включить ant-internal beta `cli-internal-2026-02-09` |
| `KOU_CC_OAUTH` | `0` | Включить `oauth-2025-04-20` beta (для OAuth-подписчиков) |

### Тесты

```bash
cargo test
```

Интеграционные тесты лежат в `tests/`:
- `integration.rs` — основной роутинг и трансляция.
- `auth_integration.rs` — setup / login / API-keys.
- `error_cases.rs` — ошибки апстрима, валидация.
- `routing_advanced.rs` — приоритеты, fallback, alias'ы.

---

## HTTP API

Все ответы — JSON, кроме SSE-стримов (`text/event-stream`) и аудио
(`audio/*`). Любой роут может вернуть `_kou_router` отладочный блок (для
не-стрим JSON) или `x-kou-debug` заголовок (для стрима).

Сквозные хедеры:
- `x-request-id` / `x-client-request-id` — переиспользуется или генерится
  UUID v4, всегда возвращается клиенту.
- `x-ratelimit-*`, `retry-after` — пробрасываются с апстрима.

### Health

| Метод | Путь       | Описание |
|-------|------------|----------|
| GET   | `/health`  | `{ "ok": true, "service": "kou-router" }` |

### Inference / proxy (OpenAI-совместимые)

Все принимают произвольный JSON, читают `model`, выбирают provider+account и
проксируют. Поддерживают streaming (`"stream": true`).

| Метод | Путь | Эндпоинт | Описание |
|-------|------|----------|----------|
| GET   | `/v1`                       | — | Список моделей (alias на `/v1/models`) |
| GET   | `/v1/models`                | — | Список всех моделей всех включённых провайдеров |
| POST  | `/v1/chat/completions`      | `chat.completions` | OpenAI Chat Completions |
| POST  | `/v1/completions`           | `completions` | OpenAI legacy completions |
| POST  | `/v1/messages`              | `messages` | Anthropic Messages |
| POST  | `/v1/messages/count_tokens` | — | **Сырой** прокси `messages/count_tokens` без трансляции |
| POST  | `/v1/responses`             | `responses` | OpenAI Responses API |
| POST  | `/v1/responses/{*path}`     | `responses` | Подэндпоинты Responses (e.g. `/cancel`) |
| POST  | `/v1/api/chat`              | `ollama.chat` | Ollama-style chat |
| GET   | `/v1/embeddings`            | — | Список моделей с capability=embeddings |
| POST  | `/v1/embeddings`            | `embeddings` | Эмбеддинги |
| GET   | `/v1/images/generations`    | — | Список image-моделей |
| POST  | `/v1/images/generations`    | `images.generations` | Генерация картинок |
| GET   | `/v1/music/generations`     | — | Список music-моделей |
| POST  | `/v1/music/generations`     | `music.generations` | Генерация музыки |
| GET   | `/v1/videos/generations`    | — | Список video-моделей |
| POST  | `/v1/videos/generations`    | `videos.generations` | Генерация видео |
| POST  | `/v1/moderations`           | `moderations` | Модерация |
| POST  | `/v1/rerank`                | `rerank` | Rerank-модели (Cohere/Voyage style) |
| GET   | `/v1/search`                | — | Список search-провайдеров |
| POST  | `/v1/search`                | `search` | Web-search (Tavily/Exa/Serper/Brave/Perplexity) |
| POST  | `/v1/audio/speech`          | `audio.speech` | TTS, возвращает бинарный аудиопоток |
| POST  | `/v1/audio/transcriptions`  | `audio.transcriptions` | STT, multipart-form-data |

**Аутентификация прокси.** Если включён `require_auth` (см. ниже), нужен
`Authorization: Bearer <api_key>` или `x-api-key: <api_key>` (Claude Code).
Иначе анонимно.

### Управление провайдерами (`/api/providers`)

| Метод | Путь | Описание |
|-------|------|----------|
| GET   | `/api/providers`           | Все `provider_connection` |
| POST  | `/api/providers`           | Создать вручную |
| GET   | `/api/providers/presets`   | Список встроенных пресетов |
| POST  | `/api/providers/import`    | Импортировать пресет (см. список ниже) |

**Body для `import`:**
```json
{
  "preset_id": "openai",
  "name": "My OpenAI",
  "api_key": "sk-...",
  "model_prefix": "openai/",
  "enabled": true,
  "priority": 100,
  "rate_limit_protection": true
}
```

### Provider accounts (`/api/provider-accounts`) — **Management auth**

| Метод | Путь | Описание |
|-------|------|----------|
| GET    | `/api/provider-accounts?provider_connection_id=...` | Список аккаунтов в провайдере |
| POST   | `/api/provider-accounts`                            | Создать аккаунт (api_key или oauth-стаб) |
| POST   | `/api/provider-accounts/oauth/start`                | Стартовать OAuth-авторизацию |
| POST   | `/api/provider-accounts/oauth/callback`             | Завершить OAuth (`state` + `code`) |
| POST   | `/api/provider-accounts/{id}/refresh`               | Принудительно обновить токен |
| POST   | `/api/provider-accounts/{id}/enable`                | Включить |
| POST   | `/api/provider-accounts/{id}/disable`               | Выключить |
| DELETE | `/api/provider-accounts/{id}`                       | Удалить |

**OAuth start body:**
```json
{
  "provider_connection_id": "uuid",
  "provider_account_id": "uuid (optional, для re-auth)",
  "redirect_uri": "http://localhost:1455/auth/callback"
}
```

Возвращает `{ session, authorization_url }`. Дальше клиент открывает URL в
браузере, ловит `code`+`state` на своём редиректе и шлёт их в `/oauth/callback`.

### Combos / Aliases / Settings / Ratelimits

| Метод | Путь | Auth | Описание |
|-------|------|------|----------|
| GET   | `/api/combos`        | — | Список комбинаций (provider+model) |
| POST  | `/api/combos`        | — | Создать комбо |
| GET   | `/api/models/alias`  | — | Список алиасов моделей |
| POST  | `/api/models/alias`  | — | Upsert алиаса (`{ "alias": "...", "target": "provider/model" }`) |
| GET   | `/api/settings`      | — | Получить settings JSON |
| POST  | `/api/settings`      | — | Сохранить settings (private/public splits) |
| GET   | `/api/ratelimits`    | — | Снимок rate-limit tracker'а по всем (provider, model) |

### Auth (публичные)

| Метод | Путь | Описание |
|-------|------|----------|
| GET   | `/api/auth/status` | `{ auth_required, setup_complete }` |
| POST  | `/api/auth/setup`  | Первичная настройка: задать пароль админа, включить auth, сгенерить JWT-secret |
| POST  | `/api/auth/login`  | `{ password }` → выставляет cookie `kou_auth=<JWT>; HttpOnly; SameSite=Lax; Max-Age=86400` |
| POST  | `/api/auth/logout` | Стирает cookie |

`setup` принимает `{ "password": "min 8 chars" }` и срабатывает только пока
`admin_password_hash` пустой.

### API keys (`/api/keys`) — **Management auth**

| Метод | Путь | Описание |
|-------|------|----------|
| GET    | `/api/keys`        | Список ключей (без секретов) |
| POST   | `/api/keys`        | Создать ключ. Возвращает `key` ровно один раз |
| DELETE | `/api/keys/{id}`   | Отозвать |

**Создание:**
```json
{ "name": "claude-code-laptop", "allowed_models": ["openai/gpt-4o-mini", "anthropic/claude-*"] }
```

`allowed_models` — белый список (точные имена или wildcard `*`). Пустой =
любые модели.

### Auth-модель в двух словах

- **Anonymous mode** (`require_auth=false`, дефолт после `cargo run` без
  setup) — все роуты открыты.
- **Authenticated mode** (`require_auth=true` после `auth/setup`):
  - Прокси-роуты (`/v1/*`) → `ProxyAuth`: bearer / `x-api-key`.
  - Управляющие (`/api/keys`, `/api/provider-accounts`) → `ManagementAuth`:
    JWT-cookie `kou_auth` **или** bearer/`x-api-key`.

---

## Встроенные провайдер-пресеты

`GET /api/providers/presets` возвращает все эти ID — их можно скармливать в
`/api/providers/import`:

**OpenAI-style API key:**
`openai`, `anthropic`, `openrouter`, `deepseek`, `groq`, `xai`, `mistral`,
`together`, `fireworks`, `cohere`, `nvidia`, `nebius`, `hyperbolic`,
`huggingface`, `vertex`, `alibaba`, `cloudflare-ai`, `aimlapi`,
`pollinations`, `glm`, `kimi`

**Search API:**
`serper-search`, `brave-search`, `exa-search`, `tavily-search`,
`perplexity-search`

**OAuth:**
`claude-oauth`, `antigravity`, `codex`, `github-copilot`

---

## Структура проекта

```
src/
├── main.rs             # entrypoint, init_db + axum::serve
├── lib.rs              # реэкспорты модулей
├── app.rs              # build_app: router + TraceLayer + CORS
├── routes.rs           # все HTTP-хендлеры и AppState
├── service.rs          # RouterService — выбор провайдера, ретраи, fallback
├── upstream.rs         # HTTP-клиент к апстриму, passthrough headers
├── repository.rs       # SQLite репо (provider_connections, accounts,
│                       #   api_keys, settings, combos, aliases, oauth_sessions)
├── db.rs               # init_db, миграции
├── models.rs           # доменные типы + EndpointKind
├── presets.rs          # встроенные пресеты провайдеров
├── error.rs            # AppError, классификация upstream-ошибок
├── search.rs           # web-search адаптеры
├── audio.rs            # TTS / STT (multipart)
├── auth/
│   ├── mod.rs          # реэкспорты, AuthContext
│   ├── api_key.rs      # генерация/хеширование (sha256)
│   ├── jwt.rs          # подпись/проверка JWT (HS256)
│   ├── middleware.rs   # ProxyAuth + ManagementAuth extractor'ы
│   ├── models.rs       # AuthStatus, LoginRequest, ApiKeyRecord, ...
│   └── password.rs     # argon2
├── oauth/
│   ├── mod.rs          # PKCE, state, парсинг JWT
│   ├── service.rs      # OAuthService — start/complete/refresh
│   ├── claude.rs       # Anthropic Console OAuth
│   └── codex.rs        # ChatGPT/Codex OAuth
├── translate/
│   ├── mod.rs          # реэкспорты
│   ├── registry.rs     # выбор адаптера по (src_protocol, dst_protocol)
│   ├── common.rs       # хелперы (роли, контент-блоки, tools)
│   ├── format.rs       # детектирование формата ответа
│   ├── stream.rs       # перекодировка SSE
│   ├── claude_to_openai.rs
│   ├── openai_to_claude.rs
│   ├── openai_to_gemini.rs
│   ├── gemini_to_openai.rs
│   └── ollama.rs
├── cost.rs             # ModelPricing, UsageInfo, расчёт $
├── ratelimit.rs        # RateLimitInfo, RateLimitTracker (in-memory)
├── retry.rs            # RetryConfig, exponential backoff с jitter
└── fingerprint.rs      # ClaudeCodeFingerprint — заголовки/metadata под Claude Code
```

---

## Скрипты

### `scripts/codex_oauth_test.py`

Self-contained Python-скрипт (только stdlib, без зависимостей) для end-to-end
проверки Codex OAuth-флоу против запущенного `kou-router`.

Что делает:
1. Поднимает локальный HTTP-сервер для callback (по умолчанию
   `http://localhost:1455/auth/callback`).
2. Через `POST /api/providers/import` импортирует пресет `codex` (или
   реюзает существующий `--provider-connection-id`).
3. Дёргает `POST /api/provider-accounts/oauth/start`, получает
   `authorization_url`.
4. Открывает URL в браузере (`webbrowser.open`).
5. Ловит callback (`code`+`state`), отправляет в
   `POST /api/provider-accounts/oauth/callback`.
6. Печатает финальный `provider_account_id` + флаги
   `has_access_token`/`has_refresh_token`.

**Запуск:**
```bash
python3 scripts/codex_oauth_test.py \
  --base-url http://127.0.0.1:20128 \
  --cookie "kou_auth=<JWT>"        # или просто "<JWT>", скрипт сам обернёт
```

**Все опции:**
| Флаг | По умолчанию | Назначение |
|---|---|---|
| `--base-url` | `http://127.0.0.1:20128` | URL роутера |
| `--cookie` | — | JWT или `kou_auth=...` для management-auth |
| `--provider-connection-id` | — | Реюзать существующее подключение |
| `--provider-name` | `Codex OAuth Test` | Имя при импорте пресета |
| `--listen-host` | `localhost` | Хост callback-сервера |
| `--listen-port` | `1455` | Порт callback-сервера |
| `--callback-path` | `/auth/callback` | Путь callback'а |
| `--no-browser` | — | Не открывать браузер, просто напечатать URL |

Exit codes: `0` ok, `1` ошибка скрипта/апстрима, `130` Ctrl+C.

---

## Лицензия

Internal / private. См. владельца репозитория.
