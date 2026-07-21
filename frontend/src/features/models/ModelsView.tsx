import { useEffect, useMemo, useState } from 'react'
import { KouModelsView, useToast } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { api } from '../../lib/api'
import type { AliasRow } from '../../lib/types'

export function ModelsView() {
  const { models, aliases, providers, demo, setAliases } = useApp()
  const toast = useToast()
  const [alias, setAlias] = useState('')
  const [target, setTarget] = useState('')
  const [lineId, setLineId] = useState('')

  const lines = useMemo(() => providers, [providers])

  useEffect(() => {
    if (!lineId && lines[0]) setLineId(lines[0].id)
  }, [lines, lineId])

  const addAlias = async () => {
    if (demo) { toast('demo mode — action disabled', 'warn'); return }
    if (!alias.trim() || !target.trim()) { toast('alias and model required', 'warn'); return }
    try {
      await api('/api/models/alias', { method: 'POST', body: { alias: alias.trim(), target: target.trim() } })
      setAlias('')
      setAliases(await api<AliasRow[]>('/api/models/alias').catch(() => aliases))
      toast('alias mapped')
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  const deleteAlias = async (name: string) => {
    if (demo) { toast('demo mode — action disabled', 'warn'); return }
    if (!confirm(`Delete alias “${name}”?`)) return
    try {
      await api(`/api/models/alias/${encodeURIComponent(name)}`, { method: 'DELETE' })
      setAliases(await api<AliasRow[]>('/api/models/alias').catch(() => aliases.filter(a => a.alias !== name)))
      toast('alias deleted')
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <KouModelsView
      models={models}
      aliases={aliases}
      lines={lines}
      alias={alias}
      target={target}
      lineId={lineId}
      onAliasChange={setAlias}
      onTargetChange={setTarget}
      onLineChange={setLineId}
      onAddAlias={() => void addAlias()}
      onDeleteAlias={name => void deleteAlias(name)}
    />
  )
}
