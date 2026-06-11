/** Decorative deterministic activity bars: same seed → same pattern,
 *  so periodic refreshes do not make the chart jitter. */
export function Spark({ seed }: { seed: string }) {
  let h = 2166136261
  for (const ch of seed) h = Math.imul(h ^ ch.charCodeAt(0), 16777619) >>> 0
  const bars = []
  for (let i = 0; i < 14; i++) {
    h = (Math.imul(h, 1103515245) + 12345) >>> 0
    const v = 3 + ((h % 1000) / 1000) * 15
    bars.push(
      <rect
        key={i}
        x={i * 6}
        y={+(18 - v).toFixed(1)}
        width={4}
        height={+v.toFixed(1)}
        rx={1.2}
        opacity={+(0.1 + (i / 14) * 0.5).toFixed(2)}
        style={{ animationDelay: `${i * 35}ms` }}
      />,
    )
  }
  return (
    <svg className="spark" viewBox="0 0 82 18" preserveAspectRatio="none">
      {bars}
    </svg>
  )
}
