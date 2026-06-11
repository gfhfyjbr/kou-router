import type { CSSProperties, ReactNode } from 'react'

export function Empty({ children, kana, style }: { children: ReactNode; kana?: string; style?: CSSProperties }) {
  return (
    <div className="empty" style={style}>
      {children}
      {kana && <span className="kana">{kana}</span>}
    </div>
  )
}
