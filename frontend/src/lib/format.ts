export function timeAgo(iso?: string | null): string {
  if (!iso) return '—'
  const d = (Date.now() - new Date(iso).getTime()) / 1000
  if (d < 0) return 'soon'
  if (d < 60) return Math.floor(d) + 's ago'
  if (d < 3600) return Math.floor(d / 60) + 'm ago'
  if (d < 86400) return Math.floor(d / 3600) + 'h ago'
  return Math.floor(d / 86400) + 'd ago'
}

export function countdown(iso: string): string | null {
  const d = (new Date(iso).getTime() - Date.now()) / 1000
  if (d <= 0) return null
  const m = Math.floor(d / 60)
  const s = Math.floor(d % 60)
  return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0')
}

export function hhmmss(d: Date, utc = false): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return utc
    ? p(d.getUTCHours()) + ':' + p(d.getUTCMinutes())
    : p(d.getHours()) + ':' + p(d.getMinutes()) + ':' + p(d.getSeconds())
}
