import {
  createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode,
} from 'react'
import { api } from '../lib/api'
import { seedDemo } from '../lib/demo'
import type {
  Account, AliasRow, ApiKeyRow, AuthStatus, ModelInfo, Preset, Provider, ViewName,
} from '../lib/types'
import { VIEW_META } from '../lib/types'

export type Mode = 'connecting' | 'live' | 'demo'

interface AppData {
  mode: Mode
  demo: boolean
  authed: boolean
  authStatus: AuthStatus | null
  providers: Provider[]
  accounts: Record<string, Account[]>
  models: ModelInfo[]
  aliases: AliasRow[]
  keys: ApiKeyRow[]
  settings: Record<string, unknown>
  presets: Preset[]
}

interface AppStore extends AppData {
  view: ViewName
  flashPid: string | null
  navigate: (v: ViewName) => void
  flashProvider: (pid: string) => void
  reload: () => Promise<void>
  setKeys: (k: ApiKeyRow[]) => void
  setAliases: (a: AliasRow[]) => void
  setSettings: (s: Record<string, unknown>) => void
}

const initial: AppData = {
  mode: 'connecting', demo: false, authed: false, authStatus: null,
  providers: [], accounts: {}, models: [], aliases: [], keys: [],
  settings: {}, presets: [],
}

const Ctx = createContext<AppStore | null>(null)

export function useApp(): AppStore {
  const s = useContext(Ctx)
  if (!s) throw new Error('useApp outside AppProvider')
  return s
}

function initialView(): ViewName {
  const h = location.hash.slice(1)
  return h in VIEW_META ? (h as ViewName) : 'overview'
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [data, setData] = useState<AppData>(initial)
  const [view, setView] = useState<ViewName>(initialView)
  const [flashPid, setFlashPid] = useState<string | null>(null)
  const demoRef = useRef(false)

  const navigate = useCallback((v: ViewName) => {
    setView(v)
    history.replaceState(null, '', '#' + v)
  }, [])

  const flashProvider = useCallback((pid: string) => {
    setFlashPid(pid)
    setTimeout(() => setFlashPid(null), 1600)
  }, [])

  const loadLive = useCallback(async (): Promise<Partial<AppData>> => {
    let authStatus: AuthStatus | null = null
    let authed = false
    let keys: ApiKeyRow[] = []
    try { authStatus = await api<AuthStatus>('/api/auth/status') } catch { authStatus = null }
    try { keys = await api<ApiKeyRow[]>('/api/keys'); authed = true } catch { keys = [] }

    const grab = async <T,>(p: string, fallback: T): Promise<T> => api<T>(p).catch(() => fallback)
    const [providers, models, aliases, settings, presets] = await Promise.all([
      grab<Provider[]>('/api/providers', []),
      grab<{ data: ModelInfo[] }>('/v1/models', { data: [] }),
      grab<AliasRow[]>('/api/models/alias', []),
      grab<Record<string, unknown>>('/api/settings', {}),
      grab<Preset[]>('/api/providers/presets', []),
    ])
    const accounts: Record<string, Account[]> = {}
    if (authed) {
      await Promise.all(providers.map(async p => {
        accounts[p.id] = await api<Account[]>(
          '/api/provider-accounts?provider_connection_id=' + encodeURIComponent(p.id),
        ).catch(() => [])
      }))
    }
    return {
      authStatus, authed, keys, providers, accounts,
      models: models.data || [], aliases, settings, presets,
    }
  }, [])

  const reload = useCallback(async () => {
    if (demoRef.current) return
    const part = await loadLive()
    setData(d => ({ ...d, ...part }))
  }, [loadLive])

  // boot: health check → live polling, or seeded demo when unreachable
  useEffect(() => {
    let timer: ReturnType<typeof setInterval> | undefined
    let cancelled = false
    ;(async () => {
      let live = false
      try {
        const ctl = new AbortController()
        const to = setTimeout(() => ctl.abort(), 2500)
        const h = await api<{ ok?: boolean }>('/health', { signal: ctl.signal })
        clearTimeout(to)
        live = !!h.ok
      } catch { live = false }
      if (cancelled) return
      if (!live) {
        demoRef.current = true
        setData(d => ({ ...d, ...seedDemo(), mode: 'demo', demo: true, authed: true }))
        return
      }
      const part = await loadLive()
      if (cancelled) return
      setData(d => ({ ...d, ...part, mode: 'live', demo: false }))
      timer = setInterval(() => { void reload() }, 30000)
    })()
    return () => { cancelled = true; if (timer) clearInterval(timer) }
  }, [loadLive, reload])

  const setKeys = useCallback((keys: ApiKeyRow[]) => setData(d => ({ ...d, keys })), [])
  const setAliases = useCallback((aliases: AliasRow[]) => setData(d => ({ ...d, aliases })), [])
  const setSettings = useCallback((settings: Record<string, unknown>) => setData(d => ({ ...d, settings })), [])

  return (
    <Ctx.Provider value={{
      ...data, view, flashPid, navigate, flashProvider, reload, setKeys, setAliases, setSettings,
    }}>
      {children}
    </Ctx.Provider>
  )
}
