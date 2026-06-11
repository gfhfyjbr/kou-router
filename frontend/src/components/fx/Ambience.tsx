import { useEffect, useRef } from 'react'
import { FINE, REDUCED } from '../../lib/env'

/** Page-wide ambience: aurora blobs, drifting embers (canvas behind the UI),
 *  click sparks (canvas above), pointer spotlight, custom cursor dot+ring,
 *  panel border-light feeding and magnetic buttons. All decoration: skipped
 *  for reduced motion; pointer-driven parts only on fine pointers. */
export function Ambience() {
  const fxA = useRef<HTMLCanvasElement>(null)
  const fxS = useRef<HTMLCanvasElement>(null)
  const spotRef = useRef<HTMLDivElement>(null)
  const dotRef = useRef<HTMLDivElement>(null)
  const ringRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (REDUCED) return
    const cvA = fxA.current!, cvS = fxS.current!
    const ctxA = cvA.getContext('2d')!, ctxS = cvS.getContext('2d')!
    const DPR = 1 // fx layers are soft glows — retina res here only burns watts
    let W = 0, H = 0
    const fit = () => {
      W = innerWidth; H = innerHeight
      for (const c of [cvA, cvS]) {
        c.width = W * DPR; c.height = H * DPR
        c.getContext('2d')!.setTransform(DPR, 0, 0, DPR, 0, 0)
      }
    }
    fit()
    addEventListener('resize', fit)

    const mouse = { x: innerWidth / 2, y: innerHeight / 2 }
    const cleanups: Array<() => void> = [() => removeEventListener('resize', fit)]

    /* ── embers + sparks ──────────────────────────────────────── */
    const EMBERS: Array<[number, number, number]> = [
      [238, 235, 225], [238, 235, 225], [43, 199, 164], [255, 99, 64], [139, 124, 246],
    ]
    interface Ember { x: number; y: number; z: number; r: number; a: number; p: number; c: [number, number, number] }
    interface SparkP { x: number; y: number; vx: number; vy: number; life: number; r: number; c: [number, number, number] }
    const newEmber = (anywhere: boolean): Ember => ({
      x: Math.random() * W,
      y: anywhere ? Math.random() * H : H + 10,
      z: 0.25 + Math.random() * 0.75,
      r: 0.6 + Math.random() * 1.7,
      a: 0.06 + Math.random() * 0.26,
      p: Math.random() * Math.PI * 2,
      c: EMBERS[(Math.random() * EMBERS.length) | 0],
    })
    const embers = Array.from({ length: 40 }, () => newEmber(true))
    const sparks: SparkP[] = []
    const onDown = (e: PointerEvent) => {
      for (let i = 0; i < 14; i++) {
        const an = Math.random() * Math.PI * 2, sp = 1.4 + Math.random() * 4
        sparks.push({
          x: e.clientX, y: e.clientY,
          vx: Math.cos(an) * sp, vy: Math.sin(an) * sp - 1.4,
          life: 1, r: 0.8 + Math.random() * 1.8,
          c: Math.random() < 0.6 ? [255, 99, 64] : [238, 235, 225],
        })
      }
    }
    addEventListener('pointerdown', onDown)
    cleanups.push(() => removeEventListener('pointerdown', onDown))

    let rafFx = 0
    let fxTimer = 0
    let framePrev = performance.now()
    let emberPrev = 0
    let sparkClean = true
    const draw = (now: number) => {
      const dt = Math.min(50, now - framePrev) / 16.7
      framePrev = now
      // embers tick at ~30 fps — half the redraws, visually identical
      if (now - emberPrev >= 31) {
        const edt = Math.min(100, now - (emberPrev || now - 33)) / 16.7
        emberPrev = now
        const ox = (mouse.x - W / 2) / W, oy = (mouse.y - H / 2) / H
        ctxA.clearRect(0, 0, W, H)
        for (let i = 0; i < embers.length; i++) {
          const m = embers[i]
          m.y -= (0.12 + 0.3 * m.z) * edt
          m.p += 0.012 * edt
          if (m.y < -12) { embers[i] = newEmber(false); continue }
          ctxA.beginPath()
          ctxA.arc(m.x + Math.sin(m.p) * 16 * m.z - ox * 44 * m.z, m.y - oy * 30 * m.z, m.r * m.z + 0.4, 0, 7)
          ctxA.fillStyle = `rgba(${m.c[0]},${m.c[1]},${m.c[2]},${(m.a * (0.6 + 0.4 * Math.sin(m.p * 2))).toFixed(3)})`
          ctxA.fill()
        }
      }
      // sparks touch their canvas only while any are alive
      if (sparks.length) {
        sparkClean = false
        ctxS.clearRect(0, 0, W, H)
        for (let i = sparks.length - 1; i >= 0; i--) {
          const s = sparks[i]
          s.x += s.vx * dt; s.y += s.vy * dt
          s.vy += 0.1 * dt; s.vx *= 0.96; s.vy *= 0.96; s.life -= 0.04 * dt
          if (s.life <= 0) { sparks.splice(i, 1); continue }
          ctxS.beginPath()
          ctxS.arc(s.x, s.y, s.r * s.life + 0.2, 0, 7)
          ctxS.shadowColor = `rgba(${s.c[0]},${s.c[1]},${s.c[2]},.85)`
          ctxS.shadowBlur = 9
          ctxS.fillStyle = `rgba(${s.c[0]},${s.c[1]},${s.c[2]},${(s.life * 0.9).toFixed(3)})`
          ctxS.fill()
          ctxS.shadowBlur = 0
        }
      } else if (!sparkClean) {
        sparkClean = true
        ctxS.clearRect(0, 0, W, H)
      }
      // sparks want 60 fps; pure ambience is happy at ~30 — halve the wakeups
      if (sparks.length) rafFx = requestAnimationFrame(draw)
      else fxTimer = window.setTimeout(() => { rafFx = requestAnimationFrame(draw) }, 33)
    }
    rafFx = requestAnimationFrame(draw)
    const onVis = () => {
      cancelAnimationFrame(rafFx); clearTimeout(fxTimer)
      if (!document.hidden) { framePrev = performance.now(); rafFx = requestAnimationFrame(draw) }
    }
    document.addEventListener('visibilitychange', onVis)
    cleanups.push(() => {
      cancelAnimationFrame(rafFx); clearTimeout(fxTimer)
      document.removeEventListener('visibilitychange', onVis)
    })

    /* ── pointer-driven layer ─────────────────────────────────── */
    if (FINE) {
      const spot = spotRef.current!, dot = dotRef.current!, ring = ringRef.current!
      document.documentElement.classList.add('no-cursor')
      cleanups.push(() => document.documentElement.classList.remove('no-cursor'))

      const rg = { x: mouse.x, y: mouse.y }, sp = { x: mouse.x, y: mouse.y }
      let moved = false, shown = false
      let wakeFollow = () => {} // assigned below, used by onMove

      const onMove = (e: PointerEvent) => {
        mouse.x = e.clientX; mouse.y = e.clientY; moved = true
        wakeFollow()
        if (!shown) {
          shown = true
          spot.style.opacity = '1'; dot.style.opacity = '1'; ring.style.opacity = '1'
        }
        const t = e.target as Element | null
        const overText = !!t?.closest?.('input,textarea,select')
        ring.classList.toggle('act', !overText && !!t?.closest?.('a,button,.chip,[data-act],[data-kid],[data-go],.y-node,#yardAdd'))
        ring.classList.toggle('txt', overText)
        dot.classList.toggle('txt', overText)
      }
      addEventListener('pointermove', onMove, { passive: true })
      cleanups.push(() => removeEventListener('pointermove', onMove))

      const paintGlow = () => {
        for (const el of document.querySelectorAll<HTMLElement>('.panel,.stat')) {
          const r = el.getBoundingClientRect()
          // the glow pseudo-layers span the whole panel, so moving --gx/--gy
          // repaints the full element — on screen-and-more tall panels
          // (logs table) that's megapixels per mousemove → single-digit fps.
          // kill the effect there instead of painting it.
          const tooTall = r.height > Math.max(1200, innerHeight * 1.2)
          if (el.classList.contains('glow-off') !== tooTall) {
            el.classList.toggle('glow-off', tooTall)
          }
          if (tooTall) continue
          if (mouse.x < r.left - 200 || mouse.x > r.right + 200 ||
              mouse.y < r.top - 200 || mouse.y > r.bottom + 200) continue
          el.style.setProperty('--gx', (mouse.x - r.left).toFixed(1) + 'px')
          el.style.setProperty('--gy', (mouse.y - r.top).toFixed(1) + 'px')
        }
      }
      let rafCur = 0
      let following = false
      const follow = () => {
        // everything converged and the pointer is still → sleep until it moves
        if (!moved &&
            Math.abs(mouse.x - rg.x) < 0.3 && Math.abs(mouse.y - rg.y) < 0.3 &&
            Math.abs(mouse.x - sp.x) < 0.3 && Math.abs(mouse.y - sp.y) < 0.3) {
          following = false
          return
        }
        rafCur = requestAnimationFrame(follow)
        dot.style.transform = `translate3d(${mouse.x}px,${mouse.y}px,0)`
        rg.x += (mouse.x - rg.x) * 0.22; rg.y += (mouse.y - rg.y) * 0.22
        ring.style.transform = `translate3d(${rg.x}px,${rg.y}px,0)`
        sp.x += (mouse.x - sp.x) * 0.055; sp.y += (mouse.y - sp.y) * 0.055
        spot.style.transform = `translate3d(${sp.x}px,${sp.y}px,0)`
        if (moved) { paintGlow(); moved = false }
      }
      wakeFollow = () => {
        if (following) return
        following = true
        rafCur = requestAnimationFrame(follow)
      }
      wakeFollow()
      cleanups.push(() => cancelAnimationFrame(rafCur))

      /* magnetic buttons */
      let mag: HTMLElement | null = null
      const onMagMove = (e: PointerEvent) => {
        const b = (e.target as Element | null)?.closest?.('.btn') as HTMLElement | null
        if (mag && mag !== b) { mag.style.transform = ''; mag = null }
        if (!b) return
        mag = b
        const r = b.getBoundingClientRect()
        b.style.transform =
          `translate(${((e.clientX - r.left - r.width / 2) * 0.16).toFixed(1)}px,` +
          `${((e.clientY - r.top - r.height / 2) * 0.26).toFixed(1)}px)`
      }
      document.addEventListener('pointermove', onMagMove, { passive: true })
      // pointer can leave the window mid-pull — without this the button
      // keeps the stale translate and sits misaligned in its row
      const onMagReset = () => {
        if (mag) { mag.style.transform = ''; mag = null }
      }
      document.addEventListener('pointerleave', onMagReset)
      window.addEventListener('blur', onMagReset)
      cleanups.push(() => {
        document.removeEventListener('pointermove', onMagMove)
        document.removeEventListener('pointerleave', onMagReset)
        window.removeEventListener('blur', onMagReset)
      })
    }

    return () => { cleanups.forEach(fn => fn()) }
  }, [])

  return (
    <>
      <div className="aurora" aria-hidden="true"><i /><i /><i /></div>
      <canvas id="fxA" ref={fxA} aria-hidden="true" />
      <div className="spot" ref={spotRef} aria-hidden="true" />
      <canvas id="fxS" ref={fxS} aria-hidden="true" />
      {FINE && !REDUCED && (
        <>
          <div className="cursor-dot" ref={dotRef} />
          <div className="cursor-ring" ref={ringRef} />
        </>
      )}
    </>
  )
}
