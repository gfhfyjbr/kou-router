import { useApp } from '../../state/store'
import { lineColor } from '../../lib/colors'
import { providerSignal } from '../../lib/providers'
import { Panel, PanelHeader, PanelTitle } from '../../components/ui/Panel'
import { Empty } from '../../components/ui/Empty'

/** Per-line semaphore posts mirroring provider health. */
export function Signals() {
  const { providers } = useApp()
  return (
    <Panel>
      <PanelHeader><PanelTitle kana="信号">SIGNALS</PanelTitle></PanelHeader>
      <div id="signals">
        {providers.length === 0 && <Empty kana="路線なし">NO LINES CONNECTED</Empty>}
        {providers.map((p, i) => {
          const sig = providerSignal(p)
          return (
            <div className="sig" key={p.id}>
              <span className="sig-post">
                {(['ok', 'warn', 'err'] as const).map(k => (
                  <span key={k} className={`lamp ${k}${sig.s === k ? ' on' : ''}`} />
                ))}
              </span>
              <span className="sig-name"><i style={{ background: lineColor(p, i) }} />{p.name || p.provider}</span>
              <span className={'sig-note mono' + (sig.s === 'warn' ? ' warn' : sig.s === 'err' ? ' err' : '')}>
                {sig.note}
              </span>
            </div>
          )
        })}
      </div>
    </Panel>
  )
}
