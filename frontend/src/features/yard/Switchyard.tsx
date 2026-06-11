import { Fragment, useEffect, useMemo, useRef, useState } from 'react'
import { useApp } from '../../state/store'
import { lineCode, lineColor } from '../../lib/colors'
import { providerSignal } from '../../lib/providers'
import { REDUCED } from '../../lib/env'
import { Panel, PanelHeader, PanelTitle } from '../../components/ui/Panel'

const CLIENTS = ['/v1/messages', '/v1/chat/completions', '/v1/responses']

/** Hero switchyard: clients → KOU core → provider lines. Tracks darken at the
 *  core and glow toward providers; hovering a node lights its line, clicking
 *  jumps to the provider card. Moving dots are spawned imperatively. */
export function Switchyard({ onAddLine }: { onAddLine: () => void }) {
  const { providers, accounts, authed, demo, mode, navigate, flashProvider } = useApp()
  const svgRef = useRef<SVGSVGElement>(null)
  const [hover, setHover] = useState<number | null>(null)

  const layout = useMemo(() => {
    const n = Math.max(providers.length, 1)
    const rows = n + 1
    const H = Math.max(300, 120 + rows * 64)
    const W = 1200, coreX = 520, coreY = H / 2, nodeX = 905
    const cliYs = CLIENTS.map((_, i) => coreY + (i - 1) * 58)
    const provYs = providers.map((_, i) => coreY + (i - (rows - 1) / 2) * 64)
    const ghostY = coreY + ((rows - 1) - (rows - 1) / 2) * 64
    return { H, W, coreX, coreY, nodeX, cliYs, provYs, ghostY }
  }, [providers])

  const { H, W, coreX, coreY, nodeX, cliYs, provYs, ghostY } = layout

  const cliPath = (y: number) => `M 60 ${y} C 240 ${y}, 300 ${coreY}, ${coreX - 46} ${coreY}`
  const provPath = (y: number) =>
    `M ${coreX + 46} ${coreY} C ${coreX + 200} ${coreY}, ${coreX + 230} ${y}, ${nodeX - 34} ${y}`

  /* moving dots along the tracks */
  useEffect(() => {
    if (REDUCED) return
    const svg = svgRef.current
    if (!svg) return
    const timers: number[] = []
    const launch = (pathId: string, color: string, r: number, durBase: number): number => {
      const dur = +(durBase + Math.random() * 1.6).toFixed(2)
      const dot = document.createElementNS('http://www.w3.org/2000/svg', 'circle')
      dot.setAttribute('r', String(r))
      dot.setAttribute('fill', color)
      dot.setAttribute('opacity', '.9')
      const am = document.createElementNS('http://www.w3.org/2000/svg', 'animateMotion')
      am.setAttribute('dur', dur + 's')
      am.setAttribute('repeatCount', '1')
      am.setAttribute('fill', 'freeze')
      // default begin resolves against the svg timeline's zero, not insertion
      // time — late dots would pop in mid-path or frozen at the end
      am.setAttribute('begin', 'indefinite')
      const mp = document.createElementNS('http://www.w3.org/2000/svg', 'mpath')
      mp.setAttribute('href', '#' + pathId)
      am.append(mp)
      dot.append(am)
      svg.append(dot)
      am.beginElement()
      timers.push(window.setTimeout(() => dot.remove(), dur * 1000 + 80))
      return dur
    }
    /* each request is one journey: a dot rides a client line into the core,
       pauses a beat for the hanko stamp, then leaves on an enabled track —
       arrivals and departures stay in sync instead of random both sides */
    const live = providers.map((p, i) => ({ p, i })).filter(x => x.p.enabled !== false)
    CLIENTS.forEach((_, i) => {
      const go = () => {
        if (!svg.isConnected) return
        const dur = launch('cli-' + i, 'rgba(238,235,225,.65)', 2.4, 2.2)
        if (live.length) {
          const t = live[(Math.random() * live.length) | 0]
          timers.push(window.setTimeout(() => {
            if (svg.isConnected) launch('trk-' + t.i, lineColor(t.p, t.i), 3, 2.6)
          }, dur * 1000 + 140))
        }
        timers.push(window.setTimeout(go, 1100 + Math.random() * 2600))
      }
      timers.push(window.setTimeout(go, Math.random() * 1800))
    })
    return () => timers.forEach(clearTimeout)
  }, [providers, mode])

  const ticker = useMemo(() => {
    const sigs = providers.map(providerSignal)
    if (sigs.some(s => s.s === 'err')) return { cls: 'err', text: 'SERVICE DISRUPTION · 運転見合わせ' }
    if (sigs.some(s => s.s === 'warn')) return { cls: 'warn', text: 'MINOR DELAYS · 遅延あり' }
    return { cls: '', text: 'ALL LINES OPERATIONAL · 全線運転' }
  }, [providers])

  return (
    <Panel className="hero">
      <PanelHeader>
        <PanelTitle kana="配線盤">SWITCHYARD</PanelTitle>
        <div className="hero-live">
          <span className="lamp ok pulse" />
          <span>{mode === 'live' ? 'LIVE FLOW' : mode === 'demo' ? 'DEMO FLOW' : '—'}</span>
        </div>
      </PanelHeader>
      <div id="yard">
        <svg
          ref={svgRef}
          viewBox={`0 0 ${W} ${H}`}
          xmlns="http://www.w3.org/2000/svg"
          className={hover != null ? 'focused' : undefined}
        >
          <defs>
            <linearGradient id="hankoGrad" x1="0" y1="0" x2="1" y2="1">
              <stop offset="0" stopColor="#ff5a36" /><stop offset="1" stopColor="#cf3517" />
            </linearGradient>
            {/* userSpaceOnUse: bbox-relative regions collapse to nothing on a
                dead-horizontal track (zero-height bbox) — the straight line
                to a provider at core height would never glow */}
            <filter id="fGlow" filterUnits="userSpaceOnUse" x={-24} y={-24} width={W + 48} height={H + 48}>
              <feGaussianBlur stdDeviation="3.5" result="b" />
              <feMerge><feMergeNode in="b" /><feMergeNode in="SourceGraphic" /></feMerge>
            </filter>
            <filter id="fBlur" filterUnits="userSpaceOnUse" x={-24} y={-24} width={W + 48} height={H + 48}>
              <feGaussianBlur stdDeviation="3" />
            </filter>
            {providers.map((p, i) => p.enabled !== false && (
              <linearGradient
                key={p.id} id={`tg-${i}`} gradientUnits="userSpaceOnUse"
                x1={coreX + 46} y1={coreY} x2={nodeX - 34} y2={provYs[i]}
              >
                <stop offset="0" style={{ stopColor: lineColor(p, i), stopOpacity: 0.08 }} />
                <stop offset=".55" style={{ stopColor: lineColor(p, i), stopOpacity: 0.3 }} />
                <stop offset="1" style={{ stopColor: lineColor(p, i), stopOpacity: 0.95 }} />
              </linearGradient>
            ))}
          </defs>

          {/* client stubs → core */}
          {CLIENTS.map((c, i) => (
            <Fragment key={c}>
              <path id={`cli-${i}`} d={cliPath(cliYs[i])} fill="none" stroke="var(--line-2)" strokeWidth={2} opacity={0.55} />
              <path d={cliPath(cliYs[i])} pathLength={100} fill="none"
                stroke="rgba(238,235,225,.4)" strokeWidth={1} strokeDasharray="2 8" opacity={0.12} />
              <circle cx={60} cy={cliYs[i]} r={4} fill="var(--ink)" stroke="var(--faint)" strokeWidth={1.5} />
              <text className="y-label" x={74} y={cliYs[i] - 9}>{c}</text>
            </Fragment>
          ))}
          <text className="y-core-cap" x={60} y={cliYs[0] - 38}>CLIENTS 入口</text>

          {/* provider tracks */}
          {providers.map((p, i) => {
            const y = provYs[i]
            const col = lineColor(p, i)
            const d = provPath(y)
            const lit = hover === i
            return (
              <Fragment key={p.id}>
                <path d={d} fill="none" stroke="var(--line-2)" strokeWidth={4} opacity={0.32} />
                {p.enabled === false ? (
                  <path id={`trk-${i}`} className="y-trk" d={d} fill="none" stroke={col} strokeWidth={1.6} opacity={0.22} />
                ) : (
                  <>
                    <path id={`glo-${i}`} className={'y-glo' + (lit ? ' lit' : '')} d={d} fill="none"
                      stroke={`url(#tg-${i})`} strokeWidth={5} filter="url(#fBlur)" opacity={0.55} />
                    <path id={`trk-${i}`} className={'y-trk' + (lit ? ' lit' : '')} d={d} fill="none"
                      stroke={`url(#tg-${i})`} strokeWidth={1.6} />
                    <path className="y-flow" d={d} pathLength={100} fill="none"
                      stroke={col} strokeWidth={1.4} strokeDasharray="2 8" opacity={0.22} />
                  </>
                )}
              </Fragment>
            )
          })}

          {/* core hanko */}
          <g>
            <g className="y-orbit">
              <circle cx={coreX} cy={coreY} r={70} fill="none" stroke="var(--line-2)" strokeWidth={1} strokeDasharray="2 10" opacity={0.7} />
            </g>
            <circle cx={coreX} cy={coreY} r={56} fill="none" stroke="var(--line-2)" strokeWidth={1} opacity={0.6} />
            <circle cx={coreX} cy={coreY} r={62} fill="none" stroke="var(--shu)" strokeWidth={1} opacity={0.16} />
            <rect
              x={coreX - 34} y={coreY - 34} width={68} height={68} rx={14}
              fill="url(#hankoGrad)" transform={`rotate(-4 ${coreX} ${coreY})`}
              stroke="rgba(255,255,255,.22)" strokeWidth={1}
            />
            <text className="y-core-glyph" x={coreX} y={coreY + 11} textAnchor="middle" transform={`rotate(-4 ${coreX} ${coreY})`}>光</text>
            <text className="y-core-cap" x={coreX} y={coreY + 62} textAnchor="middle">KOU CORE</text>
          </g>

          {/* provider nodes */}
          {providers.map((p, i) => {
            const y = provYs[i]
            const col = lineColor(p, i)
            const accs = accounts[p.id] || []
            const live = accs.filter(a => a.enabled).length
            const limited = p.rate_limited_until && new Date(p.rate_limited_until) > new Date()
            const lampCol = p.enabled === false ? 'var(--faint)' : limited ? 'var(--warn)' : 'var(--ok)'
            const sub = authed || demo
              ? `${live || accs.length || 0} ACCOUNT${(live || accs.length) === 1 ? '' : 'S'}`
              : (p.provider || '').toUpperCase()
            return (
              <g
                key={p.id}
                className="y-node"
                opacity={p.enabled === false ? 0.45 : 1}
                onMouseEnter={() => setHover(i)}
                onMouseLeave={() => setHover(h => (h === i ? null : h))}
                onClick={() => {
                  navigate('providers')
                  flashProvider(p.id)
                }}
              >
                <circle cx={nodeX} cy={y} r={25} fill="var(--ink-2)" stroke={col} strokeWidth={2} />
                <circle cx={nodeX} cy={y} r={30} fill="none" stroke={col} strokeWidth={1} opacity={0.22} />
                <text className="y-node-code" x={nodeX} y={y + 4.5} textAnchor="middle">{lineCode(p)}</text>
                <circle cx={nodeX + 19} cy={y - 18} r={4} fill={lampCol} />
                <text className="y-label-lg" x={nodeX + 44} y={y - 2}>{p.name || p.provider}</text>
                <text className="y-sub" x={nodeX + 44} y={y + 14}>{sub}</text>
              </g>
            )
          })}

          {/* ghost: add line */}
          <path
            d={`M ${coreX + 46} ${coreY} C ${coreX + 200} ${coreY}, ${coreX + 230} ${ghostY}, ${nodeX - 34} ${ghostY}`}
            fill="none" stroke="var(--line-2)" strokeWidth={1.4} strokeDasharray="3 6" opacity={0.55}
          />
          <g id="yardAdd" className="y-node" onClick={onAddLine}>
            {/* transparent (not none): an unfilled interior is invisible to
                hit-testing, hover/click would only land on the dashed stroke */}
            <circle cx={nodeX} cy={ghostY} r={25} fill="transparent" stroke="var(--faint)" strokeWidth={1.4} strokeDasharray="4 5" />
            <text className="y-node-code" x={nodeX} y={ghostY + 5} textAnchor="middle" fill="var(--faint)">+</text>
            <text className="y-sub" x={nodeX + 44} y={ghostY + 4}>ADD LINE · 新路線</text>
          </g>
        </svg>
      </div>
      <div className="hero-ticker mono">
        <span>KOU CORE · EAST GATE</span>
        <span id="tickerRight" className={ticker.cls}>{ticker.text}</span>
      </div>
    </Panel>
  )
}
