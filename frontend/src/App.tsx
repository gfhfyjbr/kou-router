import {
  AppShell,
  KouAmbience,
  MainPane,
  OverviewGrid,
  SideStack,
  ToastProvider,
  Views,
  ViewSection,
} from '@kou/ui-kit'
import { AppProvider, useApp } from './state/store'
import { Rail } from './features/layout/Rail'
import { TopBar } from './features/layout/TopBar'
import { Switchyard } from './features/yard/Switchyard'
import { StatsStrip } from './features/stats/StatsStrip'
import { DeparturesBoard } from './features/board/DeparturesBoard'
import { Signals } from './features/signals/Signals'
import { IngressChips } from './features/ingress/IngressChips'
import { ProvidersView } from './features/providers/ProvidersView'
import { KeysView } from './features/keys/KeysView'
import { ModelsView } from './features/models/ModelsView'
import { LogsView } from './features/logs/LogsView'
import { SettingsView } from './features/settings/SettingsView'
import { GateScreen } from './features/auth/AuthModal'

function OverviewView() {
  return (
    <>
      <Switchyard />
      <StatsStrip />
      <OverviewGrid>
        <DeparturesBoard />
        <SideStack>
          <Signals />
          <IngressChips />
        </SideStack>
      </OverviewGrid>
    </>
  )
}

function Shell() {
  const { view, mode, authed, authStatus } = useApp()

  // hard gate: nothing of the app renders until the admin password is entered
  // (Ambience still runs so the gate keeps the cursor, spotlight and embers)
  if (mode === 'live' && authStatus?.auth_required && !authed) {
    return (
      <>
        <KouAmbience />
        <GateScreen />
      </>
    )
  }

  return (
    <>
      <KouAmbience />
      <AppShell>
        <Rail />
        <MainPane>
          <TopBar />
          <Views logs={view === 'logs'}>
            <ViewSection viewId={view} key={view}>
              {view === 'overview' && <OverviewView />}
              {view === 'providers' && <ProvidersView />}
              {view === 'keys' && <KeysView />}
              {view === 'models' && <ModelsView />}
              {view === 'logs' && <LogsView />}
              {view === 'settings' && <SettingsView />}
            </ViewSection>
          </Views>
        </MainPane>
      </AppShell>
    </>
  )
}

export default function App() {
  return (
    <AppProvider>
      <ToastProvider>
        <Shell />
      </ToastProvider>
    </AppProvider>
  )
}
