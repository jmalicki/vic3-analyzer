interface Props {
  label: string
  /** Files or steps finished so far. Omit for work with no countable steps. */
  done?: number
  /** Total steps, when known. A missing total renders an indeterminate bar. */
  total?: number
}

/** Determinate when `total` is known; omit `done`/`total` only when the work is uncountable. */
export function ProgressBar({ label, done, total }: Props) {
  const determinate = typeof total === 'number' && total > 0 && typeof done === 'number'
  const counted = determinate ? `${done!.toLocaleString()} / ${total!.toLocaleString()}` : undefined

  return (
    <div className="progress-row">
      <progress
        aria-label={label}
        {...(determinate ? { value: done, max: total } : {})}
      />
      <small>{counted ? `${label}: ${counted}` : `${label}…`}</small>
    </div>
  )
}
