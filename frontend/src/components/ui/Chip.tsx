import type { HTMLAttributes } from 'react'

interface ChipProps extends HTMLAttributes<HTMLSpanElement> {
  mono?: boolean
}

export function Chip({ mono, className, ...rest }: ChipProps) {
  const cls = ['chip', mono ? 'mono' : '', className ?? ''].filter(Boolean).join(' ')
  return <span className={cls} {...rest} />
}
