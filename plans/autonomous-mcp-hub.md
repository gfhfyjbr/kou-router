# Autonomous MCP Hub plan

## Executive summary

`kou-router` should implement its own autonomous MCP Hub, not depend on Docker MCP Gateway as a runtime dependency. Docker MCP is useful as a reference architecture: registry, gateway, secrets, lifecycle, isolation, and client integration. The product we want is a first-party MCP control plane inside `kou-router`.

The correct external surface is not legacy SSE-only. The primary transport should be MCP Streamable HTTP:

```text
POST /mcp
Content-Type: application/json
Accept: application/json, text/event-stream
```

SSE is still useful as a response mode for streaming inside Streamable HTTP, and possibly as a legacy compatibility endpoint later. It should not be the core architecture.

The core product idea:

```text
Agents / IDEs / local tools
        |
        | one MCP endpoint + one auth model
        v
kou-router MCP Hub
        |
        | registry + profiles + policy + secrets + audit + runtime manager
        v
Managed MCP backends:
  - local stdio MCP processes
  - remote Streamable HTTP MCP servers
  - native kou-router tools
  - optional isolated workers later
```

This gives every agent a stable MCP endpoint while keeping actual tool access controlled by profiles and API-key scopes.

## Goal

Build a self-contained MCP Hub inside `kou-router` that can:

- expose one MCP server endpoint to all agents;
- aggregate tools from multiple MCP servers;
- run local stdio MCP servers as managed child processes;
- connect to remote Streamable HTTP MCP servers;
- expose shared tool profiles across many sessions;
- enforce strict auth and per-tool policy;
- store secrets safely;
- log every tool call;
- avoid dependency on Docker MCP, Docker Desktop, or any external gateway.

## Non-goals for the first version

- Do not implement a Docker-compatible catalog on day one.
- Do not mount Docker socket into `kou-router`.
- Do not execute arbitrary shell commands without policy.
- Do not make every registered tool globally available to every agent.
- Do not implement the full MCP feature matrix immediately.
- Do not run an agent loop inside the model proxy path yet.

The first version should be a robust tool gateway, not an autonomous agent framework.

## Transport decision

### Primary: Streamable HTTP

Use the current MCP Streamable HTTP model as the public endpoint:

```text
POST /mcp
```

The server can return:

- `application/json` for normal request/response;
- `text/event-stream` when a streamed response is needed.

Benefits:

- works better for multi-session servers than raw stdio;
- easier to put behind the existing `kou-router` auth/logging stack;
- maps naturally to browser/UI/admin tooling;
- supports long-running tool calls through streamed progress later;
- gives one endpoint to Claude/Cursor/Codex-style clients that support HTTP MCP.

### Compatibility: stdio bridge

Some clients still expect stdio MCP. Support that with a thin CLI bridge:

```bash
kou-router mcp-stdio --url http://127.0.0.1:20128/mcp --api-key kou_sk_...
```

The bridge should speak stdio to the client and Streamable HTTP to the local hub.

### Legacy: SSE fallback later

Old HTTP+SSE MCP can be added later if needed:

```text
GET  /mcp/sse
POST /mcp/messages
```

This is compatibility, not the main design.

## Why not just "stdio to SSE"

A raw `stdio -> SSE` pipe is not enough. It breaks down because the hub must be a real JSON-RPC broker, not a byte forwarder.

Required broker behavior:

- map request ids between client sessions and backend servers;
- virtualize multiple client sessions over one or more backend processes;
- aggregate `tools/list` across many servers;
- namespace tools to avoid collisions;
- enforce auth before every call;
- redact logs and secrets;
- handle timeouts, cancellations, restarts, and backend crashes;
- prevent unsafe parallel access to non-thread-safe stdio servers.

The right abstraction is:

```text
external MCP client
        |
        v
kou-router as MCP server
        |
        v
backend adapters as MCP clients
```

## Product model

### Server registry

An MCP server is a configured backend that can expose tools.

Supported kinds:

- `stdio`: local child process.
- `http`: remote Streamable HTTP MCP server.
- `native`: built-in `kou-router` tool provider.
- `worker`: future isolated worker runtime.

Example server config:

```json
{
  "id": "filesystem",
  "kind": "stdio",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"],
  "env": {
    "NODE_ENV": "production"
  },
  "cwd": "/Users/gfhfyjbr/Projects/Rust/kou-router",
  "enabled": true,
  "runtime_mode": "per_profile_process",
  "concurrency": 1,
  "timeout_ms": 30000
}
```

### Profiles

Profiles are the real sharing primitive. The registry can be global, but access should be profile-scoped.

Examples:

- `safe-readonly`
- `coding`
- `browser`
- `research`
- `admin`
- `project:kou-router`

Each API key or session receives one or more allowed profiles.

This avoids the bad version of "shared MCP across all sessions", where one agent accidentally gets filesystem, shell, GitHub, Linear, browser, and secrets tools from unrelated workflows.

### Tool namespace

Tools must be exposed with stable hub-level names:

```text
filesystem__read_file
filesystem__write_file
github__create_issue
linear__list_issues
browser__open
```

Store both names:

- backend name: `read_file`;
- exposed name: `filesystem__read_file`.

Allow aliases later, but the canonical exposed name should be collision-free.

### Runtime modes

Different MCP servers have different state assumptions. The hub should support explicit runtime modes:

```text
shared_process
per_profile_process
per_session_process
per_call_process
```

Defaults:

- `shared_process` only for safe/stateless/read-only providers.
- `per_profile_process` for most stdio servers.
- `per_session_process` for browser-like or stateful tools.
- `per_call_process` for high-risk tools where startup cost is acceptable.

## Policy model

MCP tool calls must use strict auth. Do not reuse best-effort proxy identity behavior from model requests.

Scopes:

```text
mcp:read
mcp:call
mcp:admin
mcp:profile:<profile_id>
mcp:tool:<tool_name>
```

Tool classifications:

```text
read
write
destructive
network
filesystem
shell
secret
external_account
browser_stateful
```

Policy examples:

- read-only tools can be enabled broadly;
- write tools require explicit profile permission;
- destructive tools require an admin policy or approval flow;
- shell tools should be disabled by default;
- filesystem tools must be root-scoped;
- tools that touch external accounts must be profile-scoped and logged;
- secrets are injected into backend runtime, never returned to the model.

## Security rules

1. Validate the incoming `Authorization` token at the hub boundary.
2. Never pass that inbound token to upstream MCP servers as their credential.
3. Inject backend credentials from the hub's secret store.
4. Redact secrets in config, logs, tool args, and tool results.
5. Cap input size, output size, and execution time per tool.
6. Default stdio concurrency to `1` unless a server is marked safe.
7. Treat local stdio servers as untrusted unless explicitly configured otherwise.
8. Keep a full audit trail for every tool call.
9. Do not expose management routes through proxy auth alone.
10. Keep MCP auth stricter than model proxy auth.

## Data model

Suggested tables:

```sql
create table mcp_servers (
  id text primary key,
  name text not null,
  kind text not null,
  transport text not null,
  command text,
  args_json text,
  url text,
  cwd text,
  env_json text,
  runtime_mode text not null,
  concurrency integer not null default 1,
  timeout_ms integer not null default 30000,
  enabled integer not null default 1,
  health text not null default 'unknown',
  config_schema_json text,
  metadata_json text,
  created_at integer not null,
  updated_at integer not null
);

create table mcp_profiles (
  id text primary key,
  name text not null,
  description text,
  enabled integer not null default 1,
  is_default integer not null default 0,
  created_at integer not null,
  updated_at integer not null
);

create table mcp_profile_servers (
  profile_id text not null,
  server_id text not null,
  enabled integer not null default 1,
  tool_prefix text,
  priority integer not null default 100,
  policy_json text,
  primary key (profile_id, server_id)
);

create table mcp_tools_cache (
  id text primary key,
  server_id text not null,
  backend_tool_name text not null,
  exposed_tool_name text not null,
  description text,
  input_schema_json text,
  output_schema_json text,
  annotations_json text,
  capabilities_hash text,
  enabled integer not null default 1,
  last_seen_at integer not null,
  updated_at integer not null,
  unique (server_id, backend_tool_name),
  unique (exposed_tool_name)
);

create table mcp_secrets (
  id text primary key,
  server_id text,
  profile_id text,
  key_name text not null,
  encrypted_value text not null,
  source text not null default 'local',
  created_at integer not null,
  updated_at integer not null
);

create table mcp_sessions (
  id text primary key,
  client_name text,
  client_version text,
  api_key_id text,
  profile_id text,
  started_at integer not null,
  last_seen_at integer not null,
  metadata_json text
);

create table mcp_tool_call_logs (
  id text primary key,
  session_id text,
  api_key_id text,
  profile_id text,
  server_id text,
  exposed_tool_name text not null,
  backend_tool_name text,
  args_redacted_json text,
  result_redacted_preview text,
  status text not null,
  duration_ms integer,
  result_bytes integer,
  error text,
  created_at integer not null
);

create table mcp_runtime_events (
  id text primary key,
  server_id text not null,
  level text not null,
  event text not null,
  message text,
  metadata_json text,
  created_at integer not null
);
```

Extend API keys with either columns:

```text
allowed_mcp_profiles_json
allowed_mcp_tools_json
mcp_policy_json
```

or a normalized `api_key_permissions` table.

## Rust module layout

Suggested structure:

```text
src/mcp/
  mod.rs
  protocol.rs       // MCP JSON-RPC types
  hub.rs            // dispatcher and tool registry view
  server.rs         // Streamable HTTP /mcp endpoint
  client.rs         // common upstream MCP client trait
  stdio.rs          // child process transport
  http.rs           // Streamable HTTP upstream transport
  native.rs         // built-in tools
  registry.rs       // DB persistence
  runtime.rs        // process lifecycle, health, restart
  policy.rs         // scopes, profiles, allow/deny
  secrets.rs        // secret lookup and injection
  redaction.rs      // args/result redaction
  sessions.rs       // session tracking
  logs.rs           // call/runtime event logging
```

Existing integration points:

- add `mcp: McpHub` to `AppState`;
- add `/mcp` protocol route;
- add `/api/mcp/*` management routes;
- add CLI mode for `mcp-stdio`;
- add DB migrations for MCP tables;
- add UI pages after the backend is usable.

## Protocol handling

The hub should implement at least:

```text
initialize
notifications/initialized
ping
tools/list
tools/call
```

Later:

```text
resources/list
resources/read
prompts/list
prompts/get
logging/setLevel
completion/complete
```

Be conservative with advanced features:

- `sampling` requires routing model requests back through a client;
- `roots` requires careful per-session/project modeling;
- `elicitation` requires an interactive approval UX.

For MVP, unsupported methods should return a clear JSON-RPC error.

## JSON-RPC broker behavior

The hub must maintain mappings:

```text
client_session_id + client_request_id
        -> backend_server_id + backend_request_id
```

For tool calls:

1. authenticate request;
2. resolve session and profile;
3. find exposed tool in profile view;
4. check policy;
5. transform exposed tool name to backend tool name;
6. acquire server runtime;
7. enqueue or dispatch call;
8. enforce timeout/cancellation;
9. redact and log args/result;
10. return MCP-compatible result.

## Stdio backend details

For each stdio MCP server:

- spawn process with configured command/args/cwd/env;
- remove unapproved inherited environment variables;
- write JSON-RPC messages to stdin;
- read JSON-RPC messages from stdout;
- capture stderr into runtime logs;
- restart on crash according to policy;
- call `initialize` once per backend runtime;
- call `tools/list` and cache the result;
- queue calls by default.

Process state:

```text
Stopped
Starting
Initializing
Ready
Degraded
Restarting
Failed
Disabled
```

## Remote HTTP backend details

For remote MCP servers:

- keep configured URL;
- authenticate with stored hub-managed credentials;
- do not forward client API keys;
- support Streamable HTTP responses;
- apply the same timeout, logging, and policy model as stdio.

## Native tools

Native tools are useful when the operation already belongs to `kou-router`.

Potential native tool groups:

- list configured models/providers;
- inspect route selection;
- read request logs;
- check provider health;
- manage MCP profiles;
- get service info.

Do not expose admin-write native tools by default.

## Management API

Suggested routes:

```text
POST /mcp
GET  /mcp

GET    /api/mcp/servers
POST   /api/mcp/servers
GET    /api/mcp/servers/:id
PATCH  /api/mcp/servers/:id
DELETE /api/mcp/servers/:id
POST   /api/mcp/servers/:id/start
POST   /api/mcp/servers/:id/stop
POST   /api/mcp/servers/:id/restart
POST   /api/mcp/servers/:id/refresh-tools

GET    /api/mcp/profiles
POST   /api/mcp/profiles
GET    /api/mcp/profiles/:id
PATCH  /api/mcp/profiles/:id
POST   /api/mcp/profiles/:id/servers
DELETE /api/mcp/profiles/:id/servers/:server_id

GET    /api/mcp/tools
PATCH  /api/mcp/tools/:id

GET    /api/mcp/sessions
GET    /api/mcp/calls
GET    /api/mcp/runtime-events

POST   /api/mcp/secrets
DELETE /api/mcp/secrets/:id
```

## UI surface

Minimum useful UI:

- servers list with health, kind, enabled state;
- server editor for command/args/env/cwd/url;
- secrets editor with redacted values;
- profiles editor;
- tools table with namespace, schema, policy tags;
- sessions view;
- tool call logs;
- runtime event logs;
- manual "refresh tools" button.

Avoid making the UI a generic JSON editor only. MCP configs are easy to misconfigure, and this should feel like a controlled operator surface.

## Result handling and token control

MCP tools can return huge outputs and hurt model quality. The hub should protect the agent from bad tool payloads.

Result controls:

- hard byte cap per call;
- schema-aware preview where possible;
- redaction before returning;
- optional result handles for large artifacts;
- summarize large tabular/text results;
- allow client to request full result explicitly;
- store full raw result only if policy allows it.

Possible response shape for oversized results:

```json
{
  "content": [
    {
      "type": "text",
      "text": "Result is 2.4MB. Returned preview only. Use mcp_result_read with handle res_123 for chunks."
    }
  ],
  "structuredContent": {
    "handle": "res_123",
    "bytes": 2401120,
    "preview_bytes": 12000,
    "truncated": true
  }
}
```

This keeps the model usable instead of dumping massive tool output into context.

## MVP implementation order

### Phase 1: Protocol skeleton

- Add MCP protocol types.
- Add `/mcp` route.
- Implement `initialize`, `ping`, `tools/list`, `tools/call`.
- Return empty tool list before backends exist.
- Add strict API-key auth for MCP.

### Phase 2: Registry and profiles

- Add DB tables.
- Add server/profile CRUD.
- Add default profile.
- Add API-key profile scopes.
- Add tool cache.

### Phase 3: Stdio runtime

- Implement child process spawning.
- Implement JSON-RPC over stdin/stdout.
- Initialize backend server.
- Cache `tools/list`.
- Route `tools/call`.
- Add per-server queue and timeout.
- Add crash handling and runtime logs.

### Phase 4: Audit and redaction

- Log all tool calls.
- Redact args/results.
- Add result size caps.
- Add runtime events.

### Phase 5: Stdio bridge

- Add `kou-router mcp-stdio`.
- Let legacy clients connect via stdio to the local HTTP hub.

### Phase 6: Remote HTTP backend

- Add Streamable HTTP upstream adapter.
- Add backend credential injection.
- Add streamed response handling.

### Phase 7: UI

- Add server/profile/tools/sessions/calls pages.
- Add health and refresh controls.

### Phase 8: Advanced policy

- Tool classification.
- Approval flow for risky tools.
- Per-project profiles.
- Native kou-router tools.
- Large-result handles.

## Verification plan

Backend tests:

- JSON-RPC method parsing;
- initialize response compatibility;
- tools/list aggregation;
- tool namespace collision handling;
- tools/call routing;
- policy allow/deny;
- timeout behavior;
- stdio process crash behavior;
- redaction behavior;
- auth required behavior.

Integration tests:

- start a tiny fake stdio MCP server;
- register it in the hub;
- call `/mcp initialize`;
- call `/mcp tools/list`;
- call `/mcp tools/call`;
- verify logs and sessions.

Manual tests:

- connect via an MCP inspector;
- connect via stdio bridge;
- test two simultaneous sessions using the same profile;
- test one session denied from an admin-only tool;
- test large result truncation.

## Main risks

### Risk: global tool chaos

If every session sees every tool, the product becomes unsafe. Fix with profiles and API-key scopes.

### Risk: stdio process state leakage

Some MCP servers are stateful. Fix with runtime modes and default `per_profile_process` or `per_session_process`.

### Risk: unsafe parallel calls

Many stdio servers are not tested under concurrent calls. Default to `concurrency = 1`.

### Risk: secret leakage

Secrets can leak through env, args, logs, tool output, or schemas. Fix with explicit secret store, env filtering, and redaction.

### Risk: huge tool outputs degrade models

Fix with size caps, previews, handles, and optional summarization.

### Risk: auth model too loose

MCP needs stricter auth than model proxy routes. Tool calls are operational actions, not just inference requests.

### Risk: protocol feature creep

Do not implement sampling/elicitation/roots before the basic tool gateway is stable.

## Final recommendation

Build `kou-router` as an autonomous MCP Hub with Streamable HTTP as the main external transport and stdio as the first backend runtime.

The strongest first product is:

```text
One local MCP endpoint.
Many managed MCP servers.
Shared registry.
Profile-scoped access.
Strict auth.
Audited tool calls.
Safe result handling.
No Docker dependency.
```

Docker MCP should stay a reference point, not a dependency. The implementation should own the registry, runtime manager, policy engine, secret handling, and protocol surface directly inside `kou-router`.
