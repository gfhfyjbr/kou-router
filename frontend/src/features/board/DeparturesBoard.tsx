import { useEffect, useMemo, useRef, useState } from 'react'
import { KouDeparturesBoard, lineColor, lineDisplayName, REDUCED, type KouBoardEvent } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { hhmmss } from '../../lib/format'

const BOARD_MAX = 9

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

/** Departures: synthetic stream in demo mode, account/provider telemetry live. */
export function DeparturesBoard() {
  const { demo, mode, providers, accounts } = useApp()
  const [demoRows, setDemoRows] = useState<KouBoardEvent[]>([])
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
        status: status as KouBoardEvent['status'],
        lat: status === 'ok' ? 280 + Math.floor(Math.random() * 2100) + 'ms' : null,
      }, ...rows].slice(0, BOARD_MAX))
      timer = window.setTimeout(push, 900 + Math.random() * 2400)
    }
    push()
    return () => { alive = false; clearTimeout(timer) }
  }, [demo])

  const liveRows = useMemo<KouBoardEvent[]>(() => {
    if (demo) return []
    const events: Array<KouBoardEvent & { ts: number }> = []
    providers.forEach((p, i) => {
      const col = lineColor(p, i)
      for (const a of accounts[p.id] || []) {
        if (a.last_used_at) events.push({
          id: `u-${a.id}`, ts: new Date(a.last_used_at).getTime(),
          time: hhmmss(new Date(a.last_used_at)).slice(0, 5),
          model: (a.label || a.remote_email || 'account') + ' · used',
          line: lineDisplayName(p), color: col,
          status: a.rate_limited_until && new Date(a.rate_limited_until) > new Date() ? 'warn' : 'ok',
          lat: null,
        })
        if (a.last_error) events.push({
          id: `e-${a.id}`, ts: a.updated_at ? new Date(a.updated_at).getTime() : 0,
          time: a.updated_at ? hhmmss(new Date(a.updated_at)).slice(0, 5) : '--:--',
          model: (a.label || 'account') + ' · ' + (a.last_error_type || 'error'),
          line: lineDisplayName(p), color: col, status: 'err', lat: null,
        })
      }
      if (p.last_error_at) events.push({
        id: `p-${p.id}`, ts: new Date(p.last_error_at).getTime(),
        time: hhmmss(new Date(p.last_error_at)).slice(0, 5),
        model: p.last_error_type || p.last_error || 'upstream error',
        line: lineDisplayName(p), color: col, status: 'err', lat: null,
      })
    })
    events.sort((a, b) => b.ts - a.ts)
    return events.slice(0, BOARD_MAX)
  }, [demo, providers, accounts])

  const rows = demo ? demoRows : liveRows

  return <KouDeparturesBoard rows={rows} mode={mode} animate={demo && !REDUCED} />
}
