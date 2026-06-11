import { useEffect, useRef, useState } from 'react'
import { useApp } from '../../state/store'
import { useToast } from '../../components/ui/toast'
import { api } from '../../lib/api'
import { lineColor } from '../../lib/colors'
import { accStatus, providerSignal } from '../../lib/providers'
import { timeAgo } from '../../lib/format'
import type { Account, Provider } from '../../lib/types'
import { Panel } from '../../components/ui/Panel'
import { Button } from '../../components/ui/Button'
import { Badge } from '../../components/ui/Badge'
import { Chip } from '../../components/ui/Chip'
import { Empty } from '../../components/ui/Empty'
import { ImportModal } from './ImportModal'
import { ConnectModal } from './ConnectModal'

function AccountRow({ a, onAction }: { a: Account; onAction: (act: string, a: Account) => void }) {
  const [lc, label] = accStatus(a)
  return (
    <div className="acc-row">
      <span className={'lamp' + (lc === 'off' ? '' : ' ' + lc)} />
      <div className="who">
        <b>{a.label || a.remote_email || '#' + String(a.id).slice(0, 8)}</b>
        {/* second line only when it adds something: an unconditional span
            duplicated the email when there's no label, and rendered an empty
            line (inflating the row) when there's no email at all */}
        {a.label && a.remote_email ? <span>{a.remote_email}</span> : null}
      </div>
      <span className="mono">{(a.auth_mode || '').toUpperCase()} · P{a.priority ?? 0}</span>
      <span className="mono">{label} · {timeAgo(a.last_used_at)}{a.proxy_url ? ' · proxy' : ''}</span>
      <div className="acc-actions">
        {a.auth_mode === 'oauth' && (
          <Button tiny data-act onClick={() => onAction('refresh', a)}>REFRESH</Button>
        )}
        <Button tiny data-act onClick={() => onAction(a.enabled ? 'disable' : 'enable', a)}>
          {a.enabled ? 'HOLD' : 'RELEASE'}
        </Button>
        <Button tiny data-act onClick={() => onAction('proxy', a)}>PROXY</Button>
        <Button tiny variant="danger" data-act onClick={() => onAction('delete', a)}>DEL</Button>
      </div>
    </div>
  )
}

function ProviderCard({
  p, i, flash, onConnect, onAction,
}: {
  p: Provider
  i: number
  flash: boolean
  onConnect: (pid: string) => void
  onAction: (act: string, a: Account) => void
}) {
  const { accounts, authed, demo } = useApp()
  const ref = useRef<HTMLDivElement>(null)
  const col = lineColor(p, i)
  const sig = providerSignal(p)
  const accs = accounts[p.id] || []

  useEffect(() => {
    if (flash) ref.current?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  }, [flash])

  return (
    <div ref={ref}>
      <Panel className={'prov-card' + (flash ? ' flash' : '')} data-pid={p.id}>
        <div className="prov-head">
          <span className="prov-dot" style={{ background: col, color: col }} />
          <span className="prov-name">{p.name || p.provider}</span>
          <Chip mono>{p.provider}</Chip>
          <Badge tone={p.enabled === false ? 'off' : sig.s === 'warn' ? 'warn' : 'on'}>
            {p.enabled === false ? 'CLOSED' : sig.s === 'warn' ? 'DELAYS' : 'IN SERVICE'}
          </Badge>
          <span className="prov-url" style={{ marginLeft: 'auto' }}>{p.base_url || ''}</span>
        </div>
        <div className="prov-meta">
          <div><dt>PREFIX</dt><dd>{p.model_prefix || '—'}</dd></div>
          <div><dt>PRIORITY</dt><dd>{p.priority ?? '—'}</dd></div>
          <div><dt>DEFAULT MODEL</dt><dd>{p.default_model || '—'}</dd></div>
          <div><dt>SIGNAL</dt><dd>{sig.note}</dd></div>
        </div>
        <div className="acc-zone">
          <div className="acc-head">
            ACCOUNTS · 口座
            {authed || demo
              ? <Button tiny variant="primary" data-act onClick={() => onConnect(p.id)}>+ CONNECT</Button>
              : <span style={{ marginLeft: 'auto' }}>sign in to manage</span>}
          </div>
          {(authed || demo) && (
            accs.length
              ? accs.map(a => <AccountRow key={a.id} a={a} onAction={onAction} />)
              : <Empty kana="口座なし" style={{ padding: 18 }}>NO ACCOUNTS</Empty>
          )}
        </div>
      </Panel>
    </div>
  )
}

export function ProvidersView() {
  const { providers, demo, flashPid, reload } = useApp()
  const toast = useToast()
  const [importOpen, setImportOpen] = useState(false)
  const [connectPid, setConnectPid] = useState<string | null>(null)

  const guardDemo = (): boolean => {
    if (demo) toast('demo mode — action disabled', 'warn')
    return demo
  }

  const onAction = async (act: string, a: Account) => {
    if (guardDemo()) return
    try {
      if (act === 'refresh') { await api(`/api/provider-accounts/${a.id}/refresh`, { method: 'POST' }); toast('token refreshed') }
      if (act === 'disable') { await api(`/api/provider-accounts/${a.id}/disable`, { method: 'POST' }); toast('account held') }
      if (act === 'enable') { await api(`/api/provider-accounts/${a.id}/enable`, { method: 'POST' }); toast('account released') }
      if (act === 'proxy') {
        const v = prompt('Upstream proxy URL (http/https/socks5), empty to clear:', a.proxy_url || '')
        if (v === null) return
        await api(`/api/provider-accounts/${a.id}/proxy`, { method: 'POST', body: { proxy_url: v.trim() || null } })
        toast('proxy updated')
      }
      if (act === 'delete') {
        if (!confirm('Delete this account?')) return
        await api(`/api/provider-accounts/${a.id}`, { method: 'DELETE' })
        toast('account deleted')
      }
      await reload()
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <>
      <div className="view-bar">
        <span className="mut">Provider connections and their authenticated accounts.</span>
        <span className="spacer" />
        <Button variant="primary" onClick={() => { if (!guardDemo()) setImportOpen(true) }}>
          + IMPORT PRESET
        </Button>
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
        {providers.length === 0 && (
          <Panel><Empty kana="路線を追加してください">NO PROVIDER LINES YET — IMPORT A PRESET</Empty></Panel>
        )}
        {providers.map((p, i) => (
          <ProviderCard
            key={p.id} p={p} i={i} flash={flashPid === p.id}
            onConnect={pid => { if (!guardDemo()) setConnectPid(pid) }}
            onAction={(act, a) => void onAction(act, a)}
          />
        ))}
      </div>
      <ImportModal open={importOpen} onClose={() => setImportOpen(false)} />
      <ConnectModal pid={connectPid} onClose={() => setConnectPid(null)} />
    </>
  )
}
