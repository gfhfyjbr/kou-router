import { useCallback, useState } from 'react'
import { api } from '../../lib/api'
import { useApp } from '../../state/store'
import { useToast } from '../../components/ui/toast'
import { Button } from '../../components/ui/Button'
import { Field, Input } from '../../components/ui/Field'

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
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')

  const go = async () => {
    setError('')
    try {
      await api('/api/auth/login', { method: 'POST', body: { password } })
      setPassword('')
      await reload()
    } catch (err) {
      setError((err as Error).message)
    }
  }

  return (
    <div
      style={{
        // transparent: the blueprint grid + grain on <body> show through;
        // z-index below the custom cursor (950) and film grain (900)
        position: 'fixed', inset: 0, zIndex: 90, display: 'grid',
        placeItems: 'center',
      }}
    >
      <div className="panel" style={{ width: 'min(340px, 86vw)', padding: '26px 24px' }}>
        <p className="mut" style={{ fontSize: '11px', letterSpacing: '.35em', margin: '0 0 2px' }}>
          改札 KOU-ROUTER
        </p>
        <h1 style={{ fontSize: '15px', letterSpacing: '.18em', margin: '0 0 16px' }}>TICKET GATE</h1>
        <Field label="ADMIN PASSWORD">
          <Input
            type="password"
            autoFocus
            autoComplete="current-password"
            value={password}
            onChange={e => setPassword(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') void go() }}
          />
        </Field>
        <Button variant="primary" style={{ width: '100%', marginTop: 12 }} onClick={() => void go()}>
          ENTER
        </Button>
        <p style={{ minHeight: 18, margin: '10px 0 0', fontSize: '12px', color: 'var(--shu, #ff4f30)' }}>
          {error}
        </p>
      </div>
    </div>
  )
}
