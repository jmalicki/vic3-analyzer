import { useEffect, useRef, type ReactNode } from 'react'

interface Props {
  title: string
  onClose: () => void
  /** While true, Escape and backdrop clicks cannot discard in-flight work. */
  locked?: boolean
  children: ReactNode
}

/** Focus-grabbing overlay dialog closed with Escape, the backdrop, or Close. */
export function Modal({ title, onClose, locked = false, children }: Props) {
  const panel = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !locked) onClose()
    }
    document.addEventListener('keydown', onKeyDown)
    panel.current?.focus()
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [locked, onClose])

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !locked) onClose()
      }}
    >
      <div
        className="modal-panel"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        ref={panel}
      >
        <div className="modal-head">
          <h3>{title}</h3>
          <button type="button" className="secondary" disabled={locked} onClick={onClose}>
            Close
          </button>
        </div>
        {children}
      </div>
    </div>
  )
}
