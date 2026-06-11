import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from 'react'

type ToastKind = 'ok' | 'warn' | 'err'
type PushToast = (msg: string, kind?: ToastKind) => void

interface ToastItem {
  id: number
  msg: string
  kind: ToastKind
  fading: boolean
}

/* a burst of actions shouldn't wallpaper the corner — oldest get evicted */
const MAX_TOASTS = 6

const ToastCtx = createContext<PushToast>(() => {})

export const useToast = () => useContext(ToastCtx)

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([])
  const seq = useRef(0)

  const push = useCallback<PushToast>((msg, kind = 'ok') => {
    const id = ++seq.current
    setToasts(t => [...t, { id, msg, kind, fading: false }].slice(-MAX_TOASTS))
    setTimeout(() => setToasts(t => t.map(x => (x.id === id ? { ...x, fading: true } : x))), 3400)
    setTimeout(() => setToasts(t => t.filter(x => x.id !== id)), 3900)
  }, [])

  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toasts">
        {toasts.map(t => (
          <div
            key={t.id}
            className={'toast' + (t.kind !== 'ok' ? ' ' + t.kind : '')}
            style={t.fading ? { opacity: 0, transition: 'opacity .4s' } : undefined}
          >
            <span className={'lamp ' + t.kind} />
            {t.msg}
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  )
}
