import { KouRail } from '@kou/ui-kit'
import { useApp } from '../../state/store'

/** Sidebar: quiet station signboards — the active stop carries a shu
 *  platform-edge light and a warm plate, all pure CSS. */
export function Rail() {
  const { view, mode, navigate } = useApp()
  return <KouRail view={view} mode={mode} onNavigate={navigate} />
}
