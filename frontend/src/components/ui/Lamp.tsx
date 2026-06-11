interface LampProps {
  tone?: 'ok' | 'warn' | 'err' | 'idle'
  pulse?: boolean
  on?: boolean
}

export function Lamp({ tone = 'idle', pulse, on }: LampProps) {
  const cls = [
    'lamp',
    tone !== 'idle' ? tone : '',
    pulse ? 'pulse' : '',
    on ? 'on' : '',
  ].filter(Boolean).join(' ')
  return <span className={cls} />
}
