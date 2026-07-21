import { useCallback, useState } from 'react'
import { KouGateScreen, useToast } from '@kou/ui-kit'
import { api } from '../../lib/api'
import { useApp } from '../../state/store'

export function useLogout() {
  const { reload } = useApp()
  const toast = useToast()
  return useCallback(async () => {
    try { await api('/api/auth/logout', { method: 'POST' }) } catch { /* best effort */ }
    toast('signed out')
    await reload()
  }, [reload, toast])
}

/**
 * Ticket gate: full-screen sign-in that replaces the whole app until the
 * admin password is entered. The password itself is set in the CLI on first
 * run (or via KOU_ROUTER_ADMIN_PASSWORD), never here.
 */
export function GateScreen() {
  const { reload } = useApp()
  const [error, setError] = useState('')

  const go = async (password: string) => {
    setError('')
    try {
      await api('/api/auth/login', { method: 'POST', body: { password } })
      await reload()
    } catch (err) {
      setError((err as Error).message)
    }
  }

  return <KouGateScreen error={error} onSubmit={go} />
}
