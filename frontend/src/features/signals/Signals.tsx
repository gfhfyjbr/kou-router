import { KouSignals, lineColor, lineDisplayName } from '@kou/ui-kit'
import { useApp } from '../../state/store'

/** Per-line account counts for the overview Signals panel. */
export function Signals() {
  const { providers, accounts } = useApp()
  return (
    <KouSignals
      rows={providers.map((p, i) => ({
        id: p.id,
        name: lineDisplayName(p),
        color: lineColor(p, i),
        accounts: (accounts[p.id] || []).length,
      }))}
    />
  )
}
