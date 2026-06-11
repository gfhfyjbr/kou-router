import type { Account, AliasRow, ApiKeyRow, LogDetail, LogRow, ModelInfo, Provider } from './types'

export interface DemoSeed {
  providers: Provider[]
  accounts: Record<string, Account[]>
  models: ModelInfo[]
  aliases: AliasRow[]
  keys: ApiKeyRow[]
  settings: Record<string, unknown>
}

export function seedDemo(): DemoSeed {
  const now = Date.now()
  return {
    providers: [
      {
        id: 'pc-claude', provider: 'claude-code', name: 'Claude Code',
        base_url: 'https://api.anthropic.com/v1', model_prefix: 'claude', priority: 10,
        enabled: true, default_model: 'claude-sonnet-4-5', rate_limited_until: null, last_error: null,
      },
      {
        id: 'pc-codex', provider: 'codex', name: 'Codex',
        base_url: 'https://chatgpt.com/backend-api/codex', model_prefix: 'codex', priority: 20,
        enabled: true, default_model: 'gpt-5-codex',
        rate_limited_until: new Date(now + 4 * 60e3 + 12e3).toISOString(), last_error: null,
      },
    ],
    accounts: {
      'pc-claude': [
        { id: 'acc-1', label: 'main', auth_mode: 'oauth', remote_email: 'kou@anthropic.dev', enabled: true, priority: 0, last_used_at: new Date(now - 42e3).toISOString(), expires_at: new Date(now + 50 * 60e3).toISOString(), rate_limited_until: null, has_refresh_token: true, proxy_url: null },
        { id: 'acc-2', label: 'backup', auth_mode: 'oauth', remote_email: 'ops@anthropic.dev', enabled: true, priority: 1, last_used_at: new Date(now - 8 * 60e3).toISOString(), expires_at: new Date(now + 110 * 60e3).toISOString(), rate_limited_until: null, has_refresh_token: true, proxy_url: 'socks5://10.0.0.7:1080' },
        { id: 'acc-3', label: 'night-shift', auth_mode: 'api_key', remote_email: null, enabled: false, priority: 2, last_used_at: new Date(now - 26 * 3600e3).toISOString(), expires_at: null, rate_limited_until: null, has_refresh_token: false, proxy_url: null },
      ],
      'pc-codex': [
        { id: 'acc-4', label: 'main', auth_mode: 'oauth', remote_email: 'kou@openai.dev', enabled: true, priority: 0, last_used_at: new Date(now - 12e3).toISOString(), expires_at: new Date(now + 30 * 60e3).toISOString(), rate_limited_until: new Date(now + 4 * 60e3).toISOString(), has_refresh_token: true, proxy_url: null },
        { id: 'acc-5', label: 'spare', auth_mode: 'oauth', remote_email: 'dev@openai.dev', enabled: true, priority: 1, last_used_at: new Date(now - 3 * 60e3).toISOString(), expires_at: new Date(now + 80 * 60e3).toISOString(), rate_limited_until: null, has_refresh_token: true, proxy_url: null },
      ],
    },
    models: [
      'claude-sonnet-4-5', 'claude-opus-4-1', 'claude-haiku-4-5',
      'gpt-5-codex', 'gpt-5', 'o4-mini', 'codex-mini-latest',
    ].map(id => ({ id, owned_by: /claude/.test(id) ? 'claude-code' : 'codex' })),
    aliases: [
      { alias: 'fast', target: 'claude-haiku-4-5' },
      { alias: 'smart', target: 'claude-opus-4-1' },
    ],
    keys: [
      { id: 'k1', name: 'cli', key_prefix: 'kou-3f8a', allowed_models: ['*'], is_active: true, usage_count: 18234, last_used_at: new Date(now - 30e3).toISOString(), created_at: new Date(now - 21 * 86400e3).toISOString() },
      { id: 'k2', name: 'ci-bot', key_prefix: 'kou-9d21', allowed_models: ['claude-haiku-4-5'], is_active: true, usage_count: 902, last_used_at: new Date(now - 3600e3).toISOString(), created_at: new Date(now - 9 * 86400e3).toISOString() },
    ],
    settings: { routing_strategy: 'priority', retry_max_attempts: 3, request_log: false },
  }
}

export function seedDemoLogs(): LogRow[] {
  const now = Date.now()
  const base: LogRow = {
    id: '', endpoint: 'chat.completions', requested_model: '', resolved_model: '',
    provider_id: 'pc-claude', provider_account_id: 'acc-1', account_label: 'main',
    api_key_name: 'cli', status: 200, error: null, attempts: 1, is_stream: true,
    input_tokens: 0, output_tokens: 0, cache_read_tokens: null, cost_usd: null,
    duration_ms: 0, created_at: '',
  }
  const rows: Array<Partial<LogRow>> = [
    { endpoint: 'messages', requested_model: 'claude/claude-opus-4-1', resolved_model: 'claude/claude-opus-4-1', input_tokens: 45_223, output_tokens: 248, duration_ms: 4_100, cache_read_tokens: 38_000 },
    { endpoint: 'responses', requested_model: 'codex/gpt-5-codex', resolved_model: 'codex/gpt-5-codex', provider_id: 'pc-codex', provider_account_id: 'acc-4', api_key_name: 'ci-bot', input_tokens: 12_034, output_tokens: 1_512, duration_ms: 12_800 },
    { endpoint: 'chat.completions', requested_model: 'smart', resolved_model: 'claude/claude-opus-4-1', input_tokens: 47_489, output_tokens: 2, duration_ms: 2_800 },
    { endpoint: 'chat.completions', requested_model: 'codex/gpt-5', resolved_model: 'codex/gpt-5', provider_id: 'pc-codex', provider_account_id: 'acc-5', account_label: 'spare', status: 502, error: 'upstream error: connection reset by peer', attempts: 3, is_stream: false, input_tokens: null, output_tokens: null, duration_ms: 954 },
    { endpoint: 'messages', requested_model: 'fast', resolved_model: 'claude/claude-haiku-4-5', input_tokens: 1_204, output_tokens: 388, duration_ms: 1_600 },
    { endpoint: 'chat.completions', requested_model: 'claude/claude-sonnet-4-5', resolved_model: 'claude/claude-sonnet-4-5', provider_account_id: 'acc-2', account_label: 'backup', status: 429, error: 'rate limit exceeded, retry after 42s', attempts: 2, is_stream: false, input_tokens: null, output_tokens: null, duration_ms: 620 },
    { endpoint: 'responses', requested_model: 'codex/codex-mini-latest', resolved_model: 'codex/codex-mini-latest', provider_id: 'pc-codex', provider_account_id: 'acc-4', input_tokens: 44_918, output_tokens: 328, duration_ms: 7_800 },
    { endpoint: 'embeddings', requested_model: 'text-embedding-3-large', resolved_model: 'text-embedding-3-large', provider_id: 'pc-codex', is_stream: false, input_tokens: 812, output_tokens: 0, duration_ms: 310 },
  ]
  return rows.map((over, i) => ({
    ...base, ...over,
    id: 'demo-' + i,
    created_at: new Date(now - i * 47e3 - 8e3).toISOString(),
  }))
}

export function seedDemoLogDetail(row: LogRow): LogDetail {
  const body = {
    model: row.requested_model,
    stream: row.is_stream,
    messages: [{ role: 'user', content: 'Explain the kou-router switchyard in one paragraph.' }],
  }
  const ok = row.status < 400
  return {
    log: { ...row, client_body: JSON.stringify(body) },
    upstream_requests: Array.from({ length: row.attempts }, (_, i) => ({
      id: row.id + '-req-' + i,
      provider_id: row.provider_id ?? 'pc-claude',
      provider_account_id: row.provider_account_id,
      model: row.resolved_model,
      endpoint: row.endpoint,
      sequence_no: i + 1,
      raw_body: JSON.stringify({ ...body, model: row.resolved_model.split('/').pop() }),
      created_at: row.created_at,
    })),
    upstream_responses: Array.from({ length: row.attempts }, (_, i) => {
      const last = i === row.attempts - 1
      return {
        id: row.id + '-resp-' + i,
        provider_id: row.provider_id ?? 'pc-claude',
        provider_account_id: row.provider_account_id,
        model: row.resolved_model,
        endpoint: row.endpoint,
        sequence_no: i + 1,
        upstream_status: last && ok ? 200 : row.status,
        raw_body: last && ok
          ? JSON.stringify({ id: 'resp_demo', model: row.resolved_model.split('/').pop(), choices: [{ message: { role: 'assistant', content: 'The switchyard fans requests out across lines…' } }], usage: { prompt_tokens: row.input_tokens, completion_tokens: row.output_tokens } })
          : JSON.stringify({ error: { message: row.error ?? 'upstream error' } }),
        created_at: row.created_at,
      }
    }),
  }
}
