import { KouSwitchyard } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { providerSignal } from '../../lib/providers'

/** Hero switchyard: clients → KOU core → provider lines. Tracks darken at the
 *  core and glow toward providers; hovering a node lights its line, clicking
 *  jumps to the provider card. Moving dots are spawned imperatively. */
export function Switchyard() {
  const { providers, accounts, authed, demo, mode, navigate, flashProvider } = useApp()
  return (
    <KouSwitchyard
      providers={providers}
      accounts={accounts}
      authed={authed}
      demo={demo}
      mode={mode}
      onNavigate={navigate}
      onFlashProvider={flashProvider}
      getSignal={providerSignal}
    />
  )
}
