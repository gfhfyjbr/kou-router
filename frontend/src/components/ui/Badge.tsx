import type { ReactNode } from 'react'

interface BadgeProps {
  tone: 'on' | 'off' | 'warn'
  children: ReactNode
}

export function Badge({ tone, children }: BadgeProps) {
  return <span className={`badge ${tone}`}>{children}</span>
}
