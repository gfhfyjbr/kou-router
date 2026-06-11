import { useEffect, type ReactNode } from 'react'
import { PanelHeader, PanelTitle } from './Panel'

interface ModalProps {
  open: boolean
  onClose: () => void
  title: ReactNode
  kana?: string
  children: ReactNode
  footer?: ReactNode
  wide?: boolean
}

/* modals can stack (detail over list) — body scroll unlocks with the last one */
let scrollLocks = 0

/** Liquid-glass dialog with backdrop blur. Closes on Esc / backdrop click.
 *  While open, the page behind is scroll-locked. */
export function Modal({ open, onClose, title, kana, children, footer, wide }: ModalProps) {
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    addEventListener('keydown', onKey)
    return () => removeEventListener('keydown', onKey)
  }, [open, onClose])

  useEffect(() => {
    if (!open) return
    scrollLocks++
    document.body.style.overflow = 'hidden'
    return () => {
      if (--scrollLocks === 0) document.body.style.overflow = ''
    }
  }, [open])

  if (!open) return null
  return (
    <div className="overlay" onClick={e => { if (e.target === e.currentTarget) onClose() }}>
      <div className={'modal' + (wide ? ' wide' : '')}>
        <PanelHeader>
          <PanelTitle kana={kana}>{title}</PanelTitle>
        </PanelHeader>
        <div className="modal-b">{children}</div>
        {footer && <div className="modal-f">{footer}</div>}
      </div>
    </div>
  )
}
