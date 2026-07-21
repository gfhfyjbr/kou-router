import { KouStatsStrip, type KouStatTile } from '@kou/ui-kit'
import { useApp } from '../../state/store'

export function StatsStrip() {
  const { accounts, models, aliases, keys, authed, demo, navigate } = useApp()
  const accs = Object.values(accounts).flat()
  const visible = authed || demo
  const tiles: KouStatTile[] = [
    { label: 'ACCOUNTS', kana: '口座', value: visible ? accs.length : null, go: 'providers' },
    { label: 'API KEYS', kana: '鍵', value: visible ? keys.length : null, go: 'keys' },
    { label: 'MODELS', kana: '車両', value: models.length, go: 'models' },
    { label: 'ALIASES', kana: '別名', value: aliases.length, go: 'models' },
  ]
  return <KouStatsStrip tiles={tiles} onNavigate={navigate} />
}
