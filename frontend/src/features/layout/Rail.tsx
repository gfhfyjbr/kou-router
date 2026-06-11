import type { ReactNode } from 'react'
import { useApp, type Mode } from '../../state/store'
import { Lamp } from '../../components/ui/Lamp'
import type { ViewName } from '../../lib/types'

interface NavItem {
  view: ViewName
  label: string
  kana: string
  icon: ReactNode
}

const NAV: NavItem[] = [
  {
    view: 'overview', label: 'Overview', kana: '概況',
    icon: (
      <svg viewBox="0 0 24 24">
        <circle cx="5" cy="12" r="2" /><circle cx="19" cy="6" r="2" /><circle cx="19" cy="18" r="2" />
        <path d="M7 12h4m0 0c4 0 4-6 6-6m-6 6c4 0 4 6 6 6" />
      </svg>
    ),
  },
  {
    view: 'providers', label: 'Lines', kana: '路線',
    icon: (
      <svg viewBox="0 0 24 24">
        <path d="M4 7h16M4 17h16" /><circle cx="9" cy="7" r="2" /><circle cx="15" cy="17" r="2" />
      </svg>
    ),
  },
  {
    view: 'keys', label: 'API Keys', kana: '鍵',
    icon: (
      <svg viewBox="0 0 24 24">
        <circle cx="8" cy="14" r="4" /><path d="M11 11l8-8m-3 1l2 2m-5 1l2 2" />
      </svg>
    ),
  },
  {
    view: 'models', label: 'Models', kana: '車両',
    icon: (
      <svg viewBox="0 0 24 24">
        <rect x="3" y="9" width="18" height="7" rx="2" /><path d="M7 16v2m10-2v2M6 12.5h.01M10 12.5h8" />
      </svg>
    ),
  },
  {
    view: 'logs', label: 'Logs', kana: '記録',
    icon: (
      <svg viewBox="0 0 24 24">
        <path d="M6 3h12v18H6z" /><path d="M9.5 7.5h5.5M9.5 11h5.5M9.5 14.5h3" />
      </svg>
    ),
  },
  {
    view: 'settings', label: 'Settings', kana: '設定',
    icon: (
      <svg viewBox="0 0 24 24">
        <path d="M4 8h10m4 0h2M4 16h2m4 0h10" /><circle cx="16" cy="8" r="2" /><circle cx="8" cy="16" r="2" />
      </svg>
    ),
  },
]

const RAIL_STATUS: Record<Mode, { tone: 'ok' | 'warn' | 'idle'; text: string }> = {
  connecting: { tone: 'idle', text: 'BOOT…' },
  live: { tone: 'ok', text: 'ON LINE' },
  demo: { tone: 'warn', text: 'DEMO' },
}

/** Sidebar: quiet station signboards — the active stop carries a shu
 *  platform-edge light and a warm plate, all pure CSS. */
export function Rail() {
  const { view, mode, navigate } = useApp()

  const status = RAIL_STATUS[mode]

  return (
    <aside className="rail">
      <div className="brand">
        <div className="hanko">光</div>
        <div className="brand-t"><b>KOU</b><span>ROUTER</span></div>
      </div>
      <nav>
        <span className="nav-cap">DISPATCH · 運行</span>
        {NAV.map(item => (
          <a
            key={item.view}
            href={'#' + item.view}
            className={view === item.view ? 'active' : undefined}
            onClick={e => { e.preventDefault(); navigate(item.view) }}
          >
            {item.icon}
            <span>{item.label}</span>
            <i>{item.kana}</i>
          </a>
        ))}
      </nav>
      <div className="rail-foot">
        <Lamp tone={status.tone === 'idle' ? 'idle' : status.tone} pulse />
        <span className="mono">{status.text}</span>
        <span className="ver">kou v0.1</span>
      </div>
    </aside>
  )
}
