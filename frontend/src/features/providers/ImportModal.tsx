import { useEffect, useState } from 'react'
import { KouImportLineModal, useToast } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { api } from '../../lib/api'

/* the import flow is OAuth-only — API-key presets are hidden,
   accounts are linked through OAuth after the line is created */
const OAUTH_PRESET_IDS = ['codex', 'claude-oauth']

export function ImportModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { presets, providers, reload } = useApp()
  const toast = useToast()
  const [presetId, setPresetId] = useState('')
  const [name, setName] = useState('')
  const [prefix, setPrefix] = useState('')

  // a preset already imported as a line has nothing left to offer
  const available = presets.filter(
    p => OAUTH_PRESET_IDS.includes(p.id) && !providers.some(prov => prov.provider === p.id),
  )
  const selected = presetId || available[0]?.id || ''

  const reset = () => { setPresetId(''); setName(''); setPrefix('') }
  const close = () => { onClose(); reset() }

  useEffect(() => {
    if (open && available.length === 0) {
      toast('all presets already imported', 'warn')
      onClose()
    }
  }, [open, available.length])

  const go = async () => {
    if (!selected) return
    try {
      const body: Record<string, string> = { preset_id: selected }
      if (name.trim()) body.name = name.trim()
      if (prefix.trim()) body.model_prefix = prefix.trim()
      await api('/api/providers/import', { method: 'POST', body })
      close()
      toast('line imported')
      await reload()
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <KouImportLineModal
      open={open}
      selected={selected}
      available={available}
      name={name}
      prefix={prefix}
      onPresetChange={setPresetId}
      onNameChange={setName}
      onPrefixChange={setPrefix}
      onClose={close}
      onImport={() => void go()}
    />
  )
}
