import { useEffect, useRef, useState } from 'react'

export interface DropdownOption {
  value: string
  label: string
  hint?: string
}

interface DropdownProps {
  value: string
  options: DropdownOption[]
  onChange: (value: string) => void
  /** Tiny caption rendered before the current value (e.g. "STATUS"). */
  label?: string
}

interface MultiDropdownProps {
  values: string[]
  options: DropdownOption[]
  onChange: (values: string[]) => void
  label?: string
  /** Trigger text when nothing is selected (e.g. "* all models"). */
  emptyLabel?: string
}

/** Multi-select flavour of the dropdown: rows toggle and the sheet stays
 *  open; the trigger sums the selection up. */
export function MultiDropdown({ values, options, onChange, label, emptyLabel = 'all' }: MultiDropdownProps) {
  const [open, setOpen] = useState(false)
  const [closing, setClosing] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  const close = () => {
    setClosing(true)
    setTimeout(() => { setOpen(false); setClosing(false) }, 160)
  }

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    document.addEventListener('mousedown', onDoc)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDoc)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const toggle = (v: string) =>
    onChange(values.includes(v) ? values.filter(x => x !== v) : [...values, v])

  const summary = values.length === 0
    ? emptyLabel
    : values.length <= 2 ? values.join(', ') : values.length + ' selected'

  return (
    <div className={'dd' + (open && !closing ? ' open' : '')} ref={ref}>
      <button
        type="button"
        className="dd-btn"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => (open ? close() : setOpen(true))}
      >
        {label && <em>{label}</em>}
        <span>{summary}</span>
        <svg className="dd-chev" viewBox="0 0 24 24">
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>
      {open && (
        <div className={'dd-menu multi' + (closing ? ' out' : '')} role="listbox" aria-multiselectable>
          <button
            type="button"
            role="option"
            aria-selected={values.length === 0}
            className={'dd-item' + (values.length === 0 ? ' sel' : '')}
            onClick={() => onChange([])}
          >
            <i className="dd-dot" />
            <span>{emptyLabel}</span>
          </button>
          {options.map((o, i) => {
            const sel = values.includes(o.value)
            return (
              <button
                type="button"
                key={o.value}
                role="option"
                aria-selected={sel}
                className={'dd-item' + (sel ? ' sel' : '')}
                style={{ animationDelay: Math.min(i, 8) * 24 + 'ms' }}
                onClick={() => toggle(o.value)}
              >
                <i className="dd-dot" />
                <span>{o.label}</span>
                {o.hint && <em>{o.hint}</em>}
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}

/** Custom select: graphite trigger with a rotating chevron, dropdown sheet
 *  springs open with staggered rows. Closes on Esc / outside click. */
export function Dropdown({ value, options, onChange, label }: DropdownProps) {
  const [open, setOpen] = useState(false)
  const [closing, setClosing] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  const close = () => {
    setClosing(true)
    setTimeout(() => { setOpen(false); setClosing(false) }, 160)
  }

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    document.addEventListener('mousedown', onDoc)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDoc)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const current = options.find(o => o.value === value)

  return (
    <div className={'dd' + (open && !closing ? ' open' : '')} ref={ref}>
      <button
        type="button"
        className="dd-btn"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => (open ? close() : setOpen(true))}
      >
        {label && <em>{label}</em>}
        <span>{current?.label ?? value}</span>
        <svg className="dd-chev" viewBox="0 0 24 24">
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>
      {open && (
        <div className={'dd-menu' + (closing ? ' out' : '')} role="listbox">
          {options.map((o, i) => (
            <button
              type="button"
              key={o.value}
              role="option"
              aria-selected={o.value === value}
              className={'dd-item' + (o.value === value ? ' sel' : '')}
              style={{ animationDelay: i * 24 + 'ms' }}
              onClick={() => { onChange(o.value); close() }}
            >
              <i className="dd-dot" />
              <span>{o.label}</span>
              {o.hint && <em>{o.hint}</em>}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
