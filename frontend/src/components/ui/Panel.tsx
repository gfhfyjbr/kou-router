import type { HTMLAttributes, ReactNode } from 'react'

/** Graphite plate with metal rim, corner glints and cursor border-light. */
export function Panel({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={['panel', className ?? ''].filter(Boolean).join(' ')} {...rest} />
}

export function PanelHeader({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={['panel-h', className ?? ''].filter(Boolean).join(' ')} {...rest} />
}

interface PanelTitleProps {
  children: ReactNode
  kana?: string
}

export function PanelTitle({ children, kana }: PanelTitleProps) {
  return (
    <div className="ttl">
      <i className="sq" />
      {children}
      {kana && <span className="kana">{kana}</span>}
    </div>
  )
}
