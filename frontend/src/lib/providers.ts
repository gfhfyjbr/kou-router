import type { Account, Provider } from './types'
import { countdown, timeAgo } from './format'

export type SignalState = 'ok' | 'warn' | 'err' | 'off'

export interface ProviderSignal {
  s: SignalState
  note: string
}

export function providerSignal(p: Provider): ProviderSignal {
  const now = Date.now()
  if (p.enabled === false) return { s: 'off', note: 'line closed · 運休' }
  if (p.rate_limited_until && new Date(p.rate_limited_until).getTime() > now) {
    const c = countdown(p.rate_limited_until)
    return { s: 'warn', note: c ? 'rate limited · resets ' + c : 'rate limited' }
  }
  if (p.last_error && p.last_error_at && now - new Date(p.last_error_at).getTime() < 10 * 60e3)
    return { s: 'err', note: (p.last_error_type || 'error') + ' · ' + timeAgo(p.last_error_at) }
  return { s: 'ok', note: 'clear · 正常' }
}

export function accStatus(a: Account): [string, string] {
  if (!a.enabled) return ['off', 'OFF']
  if (a.rate_limited_until && new Date(a.rate_limited_until) > new Date()) return ['warn', 'LIMITED']
  if (a.circuit_open_until && new Date(a.circuit_open_until) > new Date()) return ['err', 'TRIPPED']
  if (a.last_error) return ['warn', 'ERR·SEEN']
  return ['ok', 'READY']
}
