import { useState } from 'react'
import { AppProvider, useApp } from './state/store'
import { ToastProvider } from './components/ui/toast'
import { Ambience } from './components/fx/Ambience'
import { Rail } from './features/layout/Rail'
import { TopBar } from './features/layout/TopBar'
import { Switchyard } from './features/yard/Switchyard'
import { StatsStrip } from './features/stats/StatsStrip'
import { DeparturesBoard } from './features/board/DeparturesBoard'
import { Signals } from './features/signals/Signals'
import { IngressChips } from './features/ingress/IngressChips'
import { ProvidersView } from './features/providers/ProvidersView'
import { ImportModal } from './features/providers/ImportModal'
import { KeysView } from './features/keys/KeysView'
import { ModelsView } from './features/models/ModelsView'
import { LogsView } from './features/logs/LogsView'
import { SettingsView } from './features/settings/SettingsView'
import { GateScreen } from './features/auth/AuthModal'

function OverviewView({ onAddLine }: { onAddLine: () => void }) {
  return (
    <>
      <Switchyard onAddLine={onAddLine} />
      <StatsStrip />
      <div className="ov-grid">
        <DeparturesBoard />
        <div className="ov-side">
          <Signals />
          <IngressChips />
        </div>
      </div>
    </>
  )
}

function Shell() {
  const { view, navigate, mode, authed, authStatus } = useApp()
  const [importOpen, setImportOpen] = useState(false)

  // hard gate: nothing of the app renders until the admin password is entered
  // (Ambience still runs so the gate keeps the cursor, spotlight and embers)
  if (mode === 'live' && authStatus?.auth_required && !authed) {
    return (
      <>
        <Ambience />
        <GateScreen />
      </>
    )
  }

  return (
    <>
      <Ambience />
      <div className="app">
        <Rail />
        <main>
          <TopBar />
          <div className={'views views-' + view}>
            <section id={'view-' + view} className="view active" key={view}>
              {view === 'overview' && (
                <OverviewView onAddLine={() => { navigate('providers'); setImportOpen(true) }} />
              )}
              {view === 'providers' && <ProvidersView />}
              {view === 'keys' && <KeysView />}
              {view === 'models' && <ModelsView />}
              {view === 'logs' && <LogsView />}
              {view === 'settings' && <SettingsView />}
            </section>
          </div>
        </main>
      </div>
      <ImportModal open={importOpen} onClose={() => setImportOpen(false)} />
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
