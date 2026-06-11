import { useEffect, useState } from 'react'
import { useApp } from '../../state/store'
import { hhmmss } from '../../lib/format'
import { VIEW_META } from '../../lib/types'
import { Button } from '../../components/ui/Button'
import { useLogout } from '../auth/AuthModal'

function useClock() {
  const [now, setNow] = useState(() => new Date())
  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 1000)
    return () => clearInterval(t)
  }, [])
  return now
}

export function TopBar() {
  const { view, mode, authed, authStatus } = useApp()
  const now = useClock()
  const logout = useLogout()
  const meta = VIEW_META[view]

  // unauthenticated visitors never reach the TopBar — the GateScreen
  // replaces the whole app — so the only auth action left is sign-out
  const showAuthBtn = !!authStatus?.auth_required && authed

  return (
    <header className="top">
      <div className="crumb">
        <h1>{meta.t}</h1>
        <span className="kana">{meta.k}</span>
      </div>
      <div className="top-right">
        <div className={'modechip' + (mode === 'live' ? ' live' : mode === 'demo' ? ' demo' : '')}>
          <span className={'lamp' + (mode === 'live' ? ' ok pulse' : mode === 'demo' ? ' warn pulse' : '')} />
          <span>{mode === 'live' ? 'LIVE' : mode === 'demo' ? 'DEMO' : 'CONNECTING'}</span>
        </div>
        <div className="clock mono">
          <span>{hhmmss(now)}</span>
          <em>UTC {hhmmss(now, true)}</em>
        </div>
        {showAuthBtn && (
          <Button onClick={() => void logout()}>SIGN OUT</Button>
        )}
      </div>
    </header>
  )
}
