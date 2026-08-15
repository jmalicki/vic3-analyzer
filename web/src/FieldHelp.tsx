import { useId, useState, type ReactNode } from 'react'

interface Props {
  label: string
  children: ReactNode
}

/** Keyboard- and touch-accessible disclosure for brief concept help. */
export function FieldHelp({ label, children }: Props) {
  const [open, setOpen] = useState(false)
  const panelId = useId()

  return (
    <span className="field-help">
      <button
        type="button"
        className="field-help-toggle"
        aria-expanded={open}
        aria-controls={panelId}
        aria-label={label}
        onClick={() => setOpen((value) => !value)}
      >
        ?
      </button>
      {open && (
        <div className="field-help-panel" id={panelId} role="region" aria-label={label}>
          {children}
        </div>
      )}
    </span>
  )
}
