import { useState } from 'react'
import { useApp } from '../../state/store'
import { useToast } from '../../components/ui/toast'
import { api } from '../../lib/api'
import { timeAgo } from '../../lib/format'
import type { ApiKeyRow } from '../../lib/types'
import { Panel, PanelHeader, PanelTitle } from '../../components/ui/Panel'
import { Button } from '../../components/ui/Button'
import { Field, Input } from '../../components/ui/Field'
import { MultiDropdown } from '../../components/ui/Dropdown'
import { Modal } from '../../components/ui/Modal'
import { Empty } from '../../components/ui/Empty'

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
    <>
      <Panel>
        <PanelHeader><PanelTitle kana="発行">ISSUE KEY</PanelTitle></PanelHeader>
        <div className="form-row">
          <Field label="NAME">
            <Input placeholder="my-app" value={name} onChange={e => setName(e.target.value)} />
          </Field>
          <Field label="ALLOWED MODELS">
            <MultiDropdown
              values={models}
              onChange={setModels}
              emptyLabel="* all models"
              options={modelOptions}
            />
          </Field>
          <Button variant="primary" onClick={() => void create()}>CREATE</Button>
        </div>
      </Panel>
      <Panel>
        <PanelHeader><PanelTitle kana="鍵一覧">ACTIVE KEYS</PanelTitle></PanelHeader>
        <div style={{ overflowX: 'auto' }}>
          <table className="table">
            <tbody>
              {!authed && !demo ? (
                <tr><td><Empty kana="要認証">SIGN IN TO MANAGE KEYS</Empty></td></tr>
              ) : keys.length === 0 ? (
                <tr><td><Empty kana="鍵なし">NO KEYS ISSUED</Empty></td></tr>
              ) : (
                <>
                  <tr>
                    <th /><th>NAME</th><th>PREFIX</th><th>MODELS</th>
                    <th>USAGE</th><th>LAST USED</th><th>ISSUED</th><th />
                  </tr>
                  {keys.map(k => (
                    <tr key={k.id}>
                      <td><span className={'lamp' + (k.is_active === false ? '' : ' ok')} /></td>
                      <td><b>{k.name}</b></td>
                      <td className="mono">{k.key_prefix}…</td>
                      <td className="mono">{(k.allowed_models || ['*']).join(', ')}</td>
                      <td className="mono">{k.usage_count ?? 0}</td>
                      <td className="mono">{timeAgo(k.last_used_at)}</td>
                      <td className="mono">{k.created_at ? new Date(k.created_at).toLocaleDateString() : '—'}</td>
                      <td style={{ textAlign: 'right' }}>
                        <Button tiny variant="danger" data-kid={k.id} onClick={() => void revoke(k)}>REVOKE</Button>
                      </td>
                    </tr>
                  ))}
                </>
              )}
            </tbody>
          </table>
        </div>
      </Panel>
      <Modal
        open={newKey !== null}
        onClose={() => setNewKey(null)}
        title="KEY ISSUED" kana="発行済"
        footer={
          <>
            <Button variant="primary" onClick={() => {
              if (newKey) void navigator.clipboard?.writeText(newKey).then(() => toast('copied'))
            }}>COPY</Button>
            <Button onClick={() => setNewKey(null)}>DONE</Button>
          </>
        }
      >
        <p className="mut" style={{ fontSize: '12.5px' }}>Copy it now — it is shown only once.</p>
        <div className="keybox">{newKey}</div>
      </Modal>
    </>
  )
}
