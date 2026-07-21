export interface Provider {
  id: string
  provider: string
  name?: string | null
  base_url?: string | null
  model_prefix?: string | null
  priority?: number | null
  enabled?: boolean
  default_model?: string | null
  rate_limited_until?: string | null
  last_error?: string | null
  last_error_type?: string | null
  last_error_at?: string | null
}

export interface Account {
  id: string
  label?: string | null
  auth_mode?: string
  remote_email?: string | null
  enabled: boolean
  priority?: number | null
  last_used_at?: string | null
  expires_at?: string | null
  rate_limited_until?: string | null
  circuit_open_until?: string | null
  has_refresh_token?: boolean
  proxy_url?: string | null
  base_url?: string | null
  protocol_format?: string | null
  supported_endpoints?: string[] | null
  last_error?: string | null
  last_error_type?: string | null
  updated_at?: string | null
}

export interface ModelInfo {
  id: string
  owned_by?: string
}

export interface AliasRow {
  alias: string
  target: string
}

export interface ApiKeyRow {
  id: string
  name: string
  key_prefix: string
  allowed_models?: string[]
  is_active?: boolean
  usage_count?: number
  last_used_at?: string | null
  created_at?: string | null
}

export interface AuthStatus {
  auth_required: boolean
  setup_complete: boolean
}

export interface Preset {
  id: string
  display_name: string
  base_url: string
}

export interface LogRow {
  id: string
  endpoint: string
  requested_model: string
  resolved_model: string
  provider_id?: string | null
  provider_account_id?: string | null
  account_label?: string | null
  api_key_name?: string | null
  status: number
  error?: string | null
  attempts: number
  is_stream: boolean
  input_tokens?: number | null
  output_tokens?: number | null
  cache_read_tokens?: number | null
  cost_usd?: number | null
  duration_ms: number
  client_body?: string | null
  created_at: string
}

export interface LogUpstreamRequest {
  id: string
  provider_id: string
  provider_account_id?: string | null
  model: string
  endpoint: string
  sequence_no: number
  raw_body: string
  created_at: string
}

export interface LogUpstreamResponse extends LogUpstreamRequest {
  upstream_status: number
}

export interface LogDetail {
  log: LogRow
  upstream_requests: LogUpstreamRequest[]
  upstream_responses: LogUpstreamResponse[]
}

export type ViewName = 'overview' | 'providers' | 'keys' | 'models' | 'logs' | 'settings'

export const VIEW_META: Record<ViewName, { t: string; k: string }> = {
  overview: { t: 'Overview', k: '概況' },
  providers: { t: 'Lines', k: '路線' },
  keys: { t: 'API Keys', k: '鍵' },
  models: { t: 'Models', k: '車両' },
  logs: { t: 'Request Logs', k: '記録' },
  settings: { t: 'Settings', k: '設定' },
}
