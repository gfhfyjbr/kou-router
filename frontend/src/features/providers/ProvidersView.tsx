import { useMemo, useState } from 'react'
import { KouProvidersView, useToast, type KouProviderAccountAction } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { api } from '../../lib/api'
import { providerSignal } from '../../lib/providers'
import type { Account, Provider } from '../../lib/types'
import { ConnectModal } from './ConnectModal'
import { CustomApiModal } from './CustomApiModal'

function isCustomShell(p: Provider): boolean {
  if (p.provider !== 'custom') return false
  const name = (p.name || '').trim().toLowerCase()
  const base = (p.base_url || '').trim().toLowerCase()
  const prefix = (p.model_prefix || '').trim()
  // shell: exact Custom API / custom.local / model_prefix "custom"
  // leftovers: hostnames + random prefixes like custom-cnw2
  return name === 'custom api' || base.includes('custom.local') || prefix === 'custom'
}

function sortProviders(list: Provider[]): Provider[] {
  const rank = (p: Provider) => {
    if (p.provider === 'codex') return 0
    if (p.provider === 'claude-oauth' || p.provider === 'claude') return 1
    if (p.provider === 'custom') return 2
    return 3
  }
  return [...list].sort((a, b) => {
    const d = rank(a) - rank(b)
    if (d !== 0) return d
    return (a.priority ?? 0) - (b.priority ?? 0)
  })
}

export function ProvidersView() {
  const { providers, accounts, authed, demo, flashPid, reload } = useApp()
  const toast = useToast()
  const [connectPid, setConnectPid] = useState<string | null>(null)
  const [customPid, setCustomPid] = useState<string | null>(null)

  // One Custom API card only; hide leftover custom *lines* from the old flow.
  const displayProviders = useMemo(() => {
    const shells = providers.filter(isCustomShell)
    const shell = shells[0]
    const rest = providers.filter(p => p.provider !== 'custom')
    return sortProviders(shell ? [...rest, shell] : rest)
  }, [providers])

  const customLine = useMemo(
    () => providers.find(isCustomShell) ?? null,
    [providers],
  )

  const guardDemo = (): boolean => {
    if (demo) toast('demo mode — action disabled', 'warn')
    return demo
  }

  const onAction = async (act: KouProviderAccountAction, a: Account) => {
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

  const getSignal = (p: Provider) => {
    if (isCustomShell(p)) {
      const n = (accounts[p.id] || []).length
      return {
        s: 'ok' as const,
        note: n ? `${n} endpoint${n === 1 ? '' : 's'} · ready` : 'no endpoints yet · connect one',
      }
    }
    return providerSignal(p)
  }

  return (
    <>
      <KouProvidersView
        providers={displayProviders}
        accounts={accounts}
        authed={authed}
        demo={demo}
        flashProviderId={flashPid}
        getSignal={getSignal}
        onConnect={pid => {
          if (guardDemo()) return
          const p = providers.find(x => x.id === pid)
          if (p && isCustomShell(p)) setCustomPid(pid)
          else setConnectPid(pid)
        }}
        onAccountAction={(act, account) => void onAction(act, account as Account)}
      />
      <ConnectModal pid={connectPid} onClose={() => setConnectPid(null)} />
      <CustomApiModal
        open={customPid !== null}
        providerId={customPid ?? customLine?.id ?? null}
        onClose={() => setCustomPid(null)}
      />
    </>
  )
}
