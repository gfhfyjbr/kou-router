import { useRef, type HTMLAttributes, type MouseEvent, type PointerEvent } from 'react'
import { FINE, REDUCED } from '../../lib/env'

interface TiltCardProps extends HTMLAttributes<HTMLDivElement> {
  /** max tilt in degrees around Y axis (X axis uses ~0.78 of it) */
  max?: number
  /** spawn an accent ripple from the click point */
  ripple?: boolean
}

/** Parallax container: 3-D tilt that follows the pointer, optional click
 *  ripple. Generic — feature cards style themselves via className. */
export function TiltCard({ max = 9, ripple = true, onClick, children, ...rest }: TiltCardProps) {
  const ref = useRef<HTMLDivElement>(null)

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!FINE || REDUCED || !ref.current) return
    const r = ref.current.getBoundingClientRect()
    const dx = (e.clientX - r.left) / r.width - 0.5
    const dy = (e.clientY - r.top) / r.height - 0.5
    ref.current.style.transform =
      `perspective(640px) rotateX(${(-dy * max * 0.78).toFixed(2)}deg) rotateY(${(dx * max).toFixed(2)}deg) translateY(-2px)`
  }

  const onOut = (e: PointerEvent<HTMLDivElement>) => {
    if (!ref.current) return
    if (e.relatedTarget instanceof Node && ref.current.contains(e.relatedTarget)) return
    ref.current.style.transform = ''
  }

  const onClickInner = (e: MouseEvent<HTMLDivElement>) => {
    if (ripple && ref.current) {
      const rect = ref.current.getBoundingClientRect()
      const span = document.createElement('span')
      span.className = 'ripple'
      const size = Math.max(rect.width, rect.height) * 2.2
      span.style.width = span.style.height = size + 'px'
      span.style.left = e.clientX - rect.left + 'px'
      span.style.top = e.clientY - rect.top + 'px'
      ref.current.append(span)
      setTimeout(() => span.remove(), 650)
    }
    onClick?.(e)
  }

  return (
    <div ref={ref} onPointerMove={onMove} onPointerOut={onOut} onClick={onClickInner} {...rest}>
      {children}
    </div>
  )
}
