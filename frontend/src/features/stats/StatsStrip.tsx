import { useEffect, useRef, useState } from 'react'
import { useApp } from '../../state/store'
import { REDUCED } from '../../lib/env'
import { TiltCard } from '../../components/ui/TiltCard'
import { Spark } from '../../components/ui/Spark'
import type { ViewName } from '../../lib/types'

function useCountUp(target: number | null, dur = 900): number | null {
  const [v, setV] = useState(0)
  const animated = useRef(false)
  useEffect(() => {
    if (target == null) return
    if (REDUCED || animated.current || target <= 0) { animated.current = true; setV(target); return }
    animated.current = true
    const t0 = performance.now()
    let raf = 0
    const step = (now: number) => {
      const k = Math.min(1, (now - t0) / dur)
      setV(Math.round(target * (1 - Math.pow(1 - k, 3))))
      if (k < 1) raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
    return () => cancelAnimationFrame(raf)
  }, [target, dur])
  return target == null ? null : v
}

interface StatTileProps {
  label: string
  kana: string
  value: number | null
  accent?: boolean
  go: ViewName
}

/** Metric tile: parallax tilt + accent kana badge + decorative spark bars.
 *  Click ripples from the pointer, then jumps to its view. */
function StatTile({ label, kana, value, accent, go }: StatTileProps) {
  const { navigate } = useApp()
  const display = useCountUp(value)
  return (
    <TiltCard
      className={'stat' + (accent ? ' accent' : '')}
      data-go={go}
      onClick={() => setTimeout(() => navigate(go), REDUCED ? 0 : 160)}
    >
      <div className="stat-head">
        <span>{label}</span>
        <span className="stat-badge kana">{kana}</span>
      </div>
      <b className="mono">{display == null ? '—' : display}</b>
      <Spark seed={label + ':' + (value ?? '—')} />
    </TiltCard>
  )
}

export function StatsStrip() {
  const { providers, accounts, models, aliases, keys, authed, demo } = useApp()
  const accs = Object.values(accounts).flat()
  const visible = authed || demo
  return (
    <div className="stats" id="stats">
      <StatTile label="LINES" kana="路線" value={providers.length} accent go="providers" />
      <StatTile label="ACCOUNTS" kana="口座" value={visible ? accs.length : null} go="providers" />
      <StatTile label="API KEYS" kana="鍵" value={visible ? keys.length : null} go="keys" />
      <StatTile label="MODELS" kana="車両" value={models.length} go="models" />
      <StatTile label="ALIASES" kana="別名" value={aliases.length} go="models" />
    </div>
  )
}
