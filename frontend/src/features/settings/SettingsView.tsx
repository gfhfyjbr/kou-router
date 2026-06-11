import { useEffect, useState } from 'react'
import { useApp } from '../../state/store'
import { useToast } from '../../components/ui/toast'
import { api } from '../../lib/api'
import { Panel, PanelHeader, PanelTitle } from '../../components/ui/Panel'
import { Button } from '../../components/ui/Button'
import { TextArea } from '../../components/ui/Field'
import { useLogout } from '../auth/AuthModal'

function GuardBox() {
  const { demo, authed, authStatus } = useApp()
  const logout = useLogout()

  if (demo) {
    return (
      <div className="pad">
        <div className="kv"><span>MODE</span><b>demo — guard preview</b></div>
        <div className="kv"><span>AUTH</span><b>enabled</b></div>
        <div className="kv"><span>SESSION</span><b>admin</b></div>
      </div>
    )
  }
  if (!authStatus) {
    return <div className="pad"><div className="kv"><span>STATUS</span><b>unreachable</b></div></div>
  }
  return (
    <div className="pad">
      <div className="kv"><span>AUTH REQUIRED</span><b>{authStatus.auth_required ? 'yes' : 'no'}</b></div>
      <div className="kv"><span>SETUP COMPLETE</span><b>{authStatus.setup_complete ? 'yes' : 'no'}</b></div>
      <div className="kv"><span>SESSION</span><b>{authed ? 'authenticated' : 'anonymous'}</b></div>
      <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
        {authed && authStatus.auth_required && (
          <Button onClick={() => void logout()}>SIGN OUT</Button>
        )}
      </div>
      <p className="mut" style={{ fontSize: '12px', marginTop: 8 }}>
        The admin password is set in the CLI on first run, or via
        KOU_ROUTER_ADMIN_PASSWORD in non-interactive environments.
      </p>
    </div>
  )
}

export function SettingsView() {
  const { settings, demo, setSettings } = useApp()
  const toast = useToast()
  const [json, setJson] = useState('')

  useEffect(() => {
    setJson(JSON.stringify(settings ?? {}, null, 2))
  }, [settings])

  const save = async () => {
    if (demo) { toast('demo mode — action disabled', 'warn'); return }
    try {
      const parsed = JSON.parse(json) as Record<string, unknown>
      await api('/api/settings', { method: 'POST', body: parsed })
      toast('settings saved')
      setSettings(await api<Record<string, unknown>>('/api/settings').catch(() => parsed))
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <div className="settings-grid">
      <Panel>
        <PanelHeader><PanelTitle kana="改札">GUARD</PanelTitle></PanelHeader>
        <GuardBox />
      </Panel>
      <Panel>
        <PanelHeader>
          <PanelTitle kana="設定">ROUTER SETTINGS</PanelTitle>
          <Button tiny variant="primary" style={{ marginLeft: 'auto' }} onClick={() => void save()}>SAVE</Button>
        </PanelHeader>
        <div className="pad">
          <TextArea className="mono" spellCheck={false} placeholder="{ }" value={json} onChange={e => setJson(e.target.value)} />
        </div>
      </Panel>
    </div>
  )
}
