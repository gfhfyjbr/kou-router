import { useEffect, useState } from 'react'
import { KouSettingsView, useToast } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { api } from '../../lib/api'
import { useLogout } from '../auth/AuthModal'

export function SettingsView() {
  const { settings, demo, authed, authStatus, setSettings } = useApp()
  const toast = useToast()
  const logout = useLogout()
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
    <KouSettingsView
      demo={demo}
      authed={authed}
      authStatus={authStatus}
      settingsJson={json}
      onSettingsJsonChange={setJson}
      onSave={() => void save()}
      onSignOut={() => void logout()}
    />
  )
}
