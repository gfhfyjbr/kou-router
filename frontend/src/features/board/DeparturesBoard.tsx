import { useEffect, useMemo, useRef, useState } from 'react'
import { useApp } from '../../state/store'
import { hhmmss } from '../../lib/format'
import { lineColor } from '../../lib/colors'
import { REDUCED } from '../../lib/env'
import { Panel, PanelHeader, PanelTitle } from '../../components/ui/Panel'
import { Empty } from '../../components/ui/Empty'

const BOARD_MAX = 9

interface BoardEvent {
  id: string
  time: string
  model: string
  line: string
  color: string
  status: 'ok' | 'warn' | 'err'
  lat: string | null
}

const DEMO_MODELS: Array<[string, string, string, number]> = [
  ['claude-sonnet-4-5', 'Claude Code', 'var(--claude)', 0.5],
  ['claude-opus-4-1', 'Claude Code', 'var(--claude)', 0.14],
  ['claude-haiku-4-5', 'Claude Code', 'var(--claude)', 0.1],
  ['gpt-5-codex', 'Codex', 'var(--codex)', 0.16],
  ['o4-mini', 'Codex', 'var(--codex)', 0.1],
]

function pickDemoModel() {
  const r = Math.random()
  let acc = 0
  for (const m of DEMO_MODELS) { acc += m[3]; if (r <= acc) return m }
  return DEMO_MODELS[0]
}

function BoardRow({ e, animate }: { e: BoardEvent; animate: boolean }) {
  return (
    <div className={'board-r' + (animate ? ' in' : '')}>
      <span className="t">{e.time}</span>
      <span className="m">{e.model}</span>
      <span className="ln"><i style={{ background: e.color }} />{e.line}</span>
      <span className={'st ' + e.status}>{e.status === 'ok' ? 'OK' : e.status === 'warn' ? 'LIM' : 'ERR'}</span>
      <span className="lat">{e.lat ?? '—'}</span>
    </div>
  )
}

/** Departures: synthetic stream in demo mode, account/provider telemetry live. */
export function DeparturesBoard() {
  const { demo, mode, providers, accounts } = useApp()
  const [demoRows, setDemoRows] = useState<BoardEvent[]>([])
  const seq = useRef(0)

  useEffect(() => {
    if (!demo) return
    let timer: number
    let alive = true
    const push = () => {
      if (!alive) return
      const [model, line, color] = pickDemoModel()
      const r = Math.random()
      const status = r > 0.965 ? 'err' : r > 0.9 ? 'warn' : 'ok'
      setDemoRows(rows => [{
        id: 'e' + ++seq.current,
        time: hhmmss(new Date()).slice(0, 5),
        model, line, color,
        status: status as BoardEvent['status'],
        lat: status === 'ok' ? 280 + Math.floor(Math.random() * 2100) + 'ms' : null,
      }, ...rows].slice(0, BOARD_MAX))
      timer = window.setTimeout(push, 900 + Math.random() * 2400)
    }
    push()
    return () => { alive = false; clearTimeout(timer) }
  }, [demo])

  const liveRows = useMemo<BoardEvent[]>(() => {
    if (demo) return []
    const events: Array<BoardEvent & { ts: number }> = []
    providers.forEach((p, i) => {
      const col = lineColor(p, i)
      for (const a of accounts[p.id] || []) {
        if (a.last_used_at) events.push({
          id: `u-${a.id}`, ts: new Date(a.last_used_at).getTime(),
          time: hhmmss(new Date(a.last_used_at)).slice(0, 5),
          model: (a.label || a.remote_email || 'account') + ' · used',
          line: p.name || p.provider, color: col,
          status: a.rate_limited_until && new Date(a.rate_limited_until) > new Date() ? 'warn' : 'ok',
          lat: null,
        })
        if (a.last_error) events.push({
          id: `e-${a.id}`, ts: a.updated_at ? new Date(a.updated_at).getTime() : 0,
          time: a.updated_at ? hhmmss(new Date(a.updated_at)).slice(0, 5) : '--:--',
          model: (a.label || 'account') + ' · ' + (a.last_error_type || 'error'),
          line: p.name || p.provider, color: col, status: 'err', lat: null,
        })
      }
      if (p.last_error_at) events.push({
        id: `p-${p.id}`, ts: new Date(p.last_error_at).getTime(),
        time: hhmmss(new Date(p.last_error_at)).slice(0, 5),
        model: p.last_error_type || p.last_error || 'upstream error',
        line: p.name || p.provider, color: col, status: 'err', lat: null,
      })
    })
    events.sort((a, b) => b.ts - a.ts)
    return events.slice(0, BOARD_MAX)
  }, [demo, providers, accounts])

  const rows = demo ? demoRows : liveRows

  return (
    <Panel>
      <PanelHeader>
        <PanelTitle kana="発車標">DEPARTURES</PanelTitle>
        <span className="mut mono" style={{ marginLeft: 'auto', fontSize: 10, letterSpacing: '.14em' }}>
          {mode === 'demo' ? 'synthetic traffic' : mode === 'live' ? 'account telemetry' : ''}
        </span>
      </PanelHeader>
      <div className="board mono">
        <div className="board-r head">
          <span className="t">TIME</span><span className="m">MODEL</span>
          <span className="ln">LINE</span><span className="st">STATUS</span><span className="lat">LAT</span>
        </div>
        {rows.length
          ? rows.map(e => <BoardRow key={e.id} e={e} animate={demo && !REDUCED} />)
          : <Empty kana="待機中">AWAITING TRAFFIC</Empty>}
      </div>
    </Panel>
  )
}
