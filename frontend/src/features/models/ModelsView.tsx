import { useState } from 'react'
import { useApp } from '../../state/store'
import { useToast } from '../../components/ui/toast'
import { api } from '../../lib/api'
import { modelColor } from '../../lib/colors'
import type { AliasRow } from '../../lib/types'
import { Panel, PanelHeader, PanelTitle } from '../../components/ui/Panel'
import { Button } from '../../components/ui/Button'
import { Field, Input } from '../../components/ui/Field'
import { Empty } from '../../components/ui/Empty'

export function ModelsView() {
  const { models, aliases, demo, setAliases } = useApp()
  const toast = useToast()
  const [alias, setAlias] = useState('')
  const [target, setTarget] = useState('')

  const addAlias = async () => {
    if (demo) { toast('demo mode — action disabled', 'warn'); return }
    if (!alias.trim() || !target.trim()) { toast('alias and target required', 'warn'); return }
    try {
      await api('/api/models/alias', { method: 'POST', body: { alias: alias.trim(), target: target.trim() } })
      setAlias(''); setTarget('')
      setAliases(await api<AliasRow[]>('/api/models/alias').catch(() => aliases))
      toast('alias mapped')
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <div className="models-grid">
      <Panel>
        <PanelHeader>
          <PanelTitle kana="車両一覧">ROLLING STOCK</PanelTitle>
          <span className="mut mono" style={{ marginLeft: 'auto', fontSize: 10 }}>{models.length} UNITS</span>
        </PanelHeader>
        <div>
          {models.length === 0 && <Empty kana="車両なし">NO MODELS VISIBLE</Empty>}
          {models.map(m => (
            <div className="model-li" key={m.id}>
              <i style={{ background: modelColor(m.id) }} />
              {m.id}
              <span>{m.owned_by || ''}</span>
            </div>
          ))}
        </div>
      </Panel>
      <Panel>
        <PanelHeader><PanelTitle kana="別名">ALIASES</PanelTitle></PanelHeader>
        <div className="form-row">
          <Field label="ALIAS">
            <Input placeholder="fast" value={alias} onChange={e => setAlias(e.target.value)} />
          </Field>
          <Field label="TARGET">
            <Input placeholder="claude-haiku-4-5" value={target} onChange={e => setTarget(e.target.value)} />
          </Field>
          <Button variant="primary" onClick={() => void addAlias()}>MAP</Button>
        </div>
        <div style={{ overflowX: 'auto' }}>
          <table className="table">
            <tbody>
              {aliases.length === 0 ? (
                <tr><td><Empty kana="別名なし">NO ALIASES</Empty></td></tr>
              ) : (
                <>
                  <tr><th>ALIAS</th><th /><th>TARGET</th></tr>
                  {aliases.map(a => (
                    <tr key={a.alias}>
                      <td className="mono" style={{ color: 'var(--shu)' }}>{a.alias}</td>
                      <td className="mut mono" style={{ textAlign: 'center' }}>→</td>
                      <td className="mono">{a.target}</td>
                    </tr>
                  ))}
                </>
              )}
            </tbody>
          </table>
        </div>
      </Panel>
    </div>
  )
}
