import { useState } from 'react'
import { KouKeysView, useToast } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { api } from '../../lib/api'
import type { ApiKeyRow } from '../../lib/types'

export function KeysView() {
  const { keys, authed, demo, setKeys, models: routedModels } = useApp()
  const toast = useToast()
  const [name, setName] = useState('')
  const [models, setModels] = useState<string[]>([])
  const [newKey, setNewKey] = useState<string | null>(null)

  const modelOptions = [...new Set(routedModels.map(m => m.id))]
    .sort()
    .map(id => ({ value: id, label: id }))

  const guardDemo = (): boolean => {
    if (demo) toast('demo mode — action disabled', 'warn')
    return demo
  }

  const create = async () => {
    if (guardDemo()) return
    if (!name.trim()) { toast('key name required', 'warn'); return }
    try {
      const created = await api<{ key: string }>('/api/keys', {
        method: 'POST',
        body: { name: name.trim(), allowed_models: models.length ? models : ['*'] },
      })
      setName(''); setModels([])
      setKeys(await api<ApiKeyRow[]>('/api/keys').catch(() => keys))
      setNewKey(created.key)
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  const revoke = async (k: ApiKeyRow) => {
    if (guardDemo()) return
    if (!confirm(`Revoke key “${k.name}”?`)) return
    try {
      await api('/api/keys/' + k.id, { method: 'DELETE' })
      toast('key revoked')
      setKeys(await api<ApiKeyRow[]>('/api/keys'))
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <KouKeysView
      keys={keys}
      authed={authed || demo}
      name={name}
      selectedModels={models}
      modelOptions={modelOptions}
      newKey={newKey}
      onNameChange={setName}
      onSelectedModelsChange={setModels}
      onCreate={() => void create()}
      onRevoke={key => void revoke(key as ApiKeyRow)}
      onCloseNewKey={() => setNewKey(null)}
      onCopyNewKey={() => {
        if (newKey) void navigator.clipboard?.writeText(newKey).then(() => toast('copied'))
      }}
    />
  )
}
