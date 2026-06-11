import { useEffect, useMemo, useState } from 'react'
import { useApp } from '../../state/store'
import { useToast } from '../../components/ui/toast'
import { api } from '../../lib/api'
import { Modal } from '../../components/ui/Modal'
import { Button } from '../../components/ui/Button'
import { Field, Input } from '../../components/ui/Field'
import { Dropdown } from '../../components/ui/Dropdown'

interface OAuthSession {
  state: string
}

/** Connect an account to a provider line: OAuth two-step or plain API key. */
export function ConnectModal({ pid, onClose }: { pid: string | null; onClose: () => void }) {
  const { providers, reload } = useApp()
  const toast = useToast()
  const [authMode, setAuthMode] = useState<'oauth' | 'api_key'>('oauth')
  const [label, setLabel] = useState('')
  const [redirect, setRedirect] = useState<string | null>(null)
  const [proxy, setProxy] = useState('')
  const [code, setCode] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [session, setSession] = useState<OAuthSession | null>(null)

  const p = useMemo(() => providers.find(x => x.id === pid), [providers, pid])
  const isClaude = /claude|anthropic/i.test((p?.provider || '') + (p?.name || ''))
  const defRedirect = isClaude
    ? 'https://console.anthropic.com/oauth/code/callback'
    : 'http://localhost:1455/auth/callback'

  const reset = () => {
    setSession(null); setCode(''); setRedirect(null); setLabel(''); setApiKey(''); setProxy('')
  }

  // codex: the router serves the localhost:1455 callback itself, so the
  // account appears without pasting anything — poll until it shows up
  useEffect(() => {
    if (!session || !pid || isClaude || authMode !== 'oauth') return
    let baseline: number | null = null
    let cancelled = false
    const tick = async () => {
      try {
        const list = await api<unknown[]>(
          '/api/provider-accounts?provider_connection_id=' + encodeURIComponent(pid),
        )
        if (cancelled) return
        if (baseline === null) { baseline = list.length; return }
        if (list.length > baseline) {
          toast('account connected ✓')
          onClose(); reset()
          await reload()
        }
      } catch { /* keep polling */ }
    }
    void tick()
    const t = setInterval(() => { void tick() }, 2500)
    return () => { cancelled = true; clearInterval(t) }
  }, [session, pid, isClaude, authMode]) // deps намеренно без onClose/reload — иначе интервал пересоздаётся каждый рендер

  const go = async () => {
    if (!pid) return
    try {
      if (authMode === 'api_key') {
        await api('/api/provider-accounts', {
          method: 'POST',
          body: {
            provider_connection_id: pid,
            label: label.trim() || null,
            auth_mode: 'api_key',
            api_key: apiKey.trim(),
          },
        })
        onClose(); reset()
        toast('account saved')
      } else if (!session) {
        const body: Record<string, string> = {
          provider_connection_id: pid,
          redirect_uri: (redirect ?? defRedirect).trim(),
        }
        if (proxy.trim()) body.proxy_url = proxy.trim()
        const r = await api<{ session: OAuthSession; authorization_url: string }>(
          '/api/provider-accounts/oauth/start', { method: 'POST', body },
        )
        setSession(r.session)
        window.open(r.authorization_url, '_blank')
        toast(isClaude
          ? 'authorize in the opened tab, then paste the code'
          : 'authorize in the opened tab — the account connects automatically')
        return
      } else {
        let c = code.trim()
        let st = session.state
        if (c.includes('#')) {
          const [cc, s] = c.split('#')
          c = cc
          if (s) st = s
        }
        await api('/api/provider-accounts/oauth/callback', { method: 'POST', body: { state: st, code: c } })
        onClose(); reset()
        toast('account connected ✓')
      }
      await reload()
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <Modal
      open={pid !== null}
      onClose={() => { onClose(); reset() }}
      title="CONNECT ACCOUNT" kana="口座接続"
      footer={
        <>
          <Button onClick={() => { onClose(); reset() }}>CANCEL</Button>
          <Button variant="primary" onClick={() => void go()}>
            {authMode === 'oauth' ? (session ? 'COMPLETE' : 'START') : 'SAVE'}
          </Button>
        </>
      }
    >
      <Field label="MODE">
        <Dropdown
          value={authMode}
          onChange={v => setAuthMode(v as 'oauth' | 'api_key')}
          options={[
            { value: 'oauth', label: 'OAuth' },
            { value: 'api_key', label: 'API key' },
          ]}
        />
      </Field>
      <Field label="LABEL">
        <Input placeholder="main" value={label} onChange={e => setLabel(e.target.value)} />
      </Field>
      {authMode === 'oauth' ? (
        <>
          <Field label="REDIRECT URI">
            <Input value={redirect ?? defRedirect} onChange={e => setRedirect(e.target.value)} />
          </Field>
          <Field label="PROXY FOR OAUTH (optional)">
            <Input placeholder="socks5://…" value={proxy} onChange={e => setProxy(e.target.value)} />
          </Field>
          {session && (
            <Field label="PASTE CODE (code or code#state)">
              <Input placeholder="ac_…" value={code} onChange={e => setCode(e.target.value)} />
            </Field>
          )}
        </>
      ) : (
        <Field label="API KEY">
          <Input placeholder="sk-…" value={apiKey} onChange={e => setApiKey(e.target.value)} />
        </Field>
      )}
    </Modal>
  )
}
