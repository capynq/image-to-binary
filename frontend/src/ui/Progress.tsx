import { useId } from 'react'
import type { Ref } from 'react'
import { Button } from './Button'

export interface ProgressProps {
  /** Visible and accessible name, for example "Rendering PNG". */
  readonly label: string
  /** Fraction in [0, 1], clamped. Omit it for work whose length is unknown. */
  readonly fraction?: number
  /** Adds a cancel action beneath the bar when present. */
  readonly onCancel?: (() => void) | undefined
  readonly cancelLabel?: string
  readonly cancelRef?: Ref<HTMLButtonElement>
}

/**
 * Determinate or indeterminate progress. A determinate bar exposes `aria-valuenow`,
 * `aria-valuemin`, `aria-valuemax` and `aria-valuetext`; an indeterminate bar omits the value so
 * assistive technology reports it as working, not as stuck at zero. The percentage is printed as
 * text, so the state is never conveyed by the bar's colour or motion alone.
 */
export function Progress({
  label,
  fraction,
  onCancel,
  cancelLabel = 'Cancel',
  cancelRef,
}: ProgressProps) {
  const labelId = useId()
  const determinate = fraction !== undefined
  const percent = determinate ? Math.round(Math.min(1, Math.max(0, fraction)) * 100) : null
  return (
    <div className="progress" data-state={determinate ? 'determinate' : 'indeterminate'}>
      <div className="progress-head">
        <span id={labelId}>{label}</span>
        {percent !== null && <span className="progress-percent">{percent}%</span>}
      </div>
      <div
        className="progress-track"
        role="progressbar"
        aria-labelledby={labelId}
        aria-valuemin={determinate ? 0 : undefined}
        aria-valuemax={determinate ? 100 : undefined}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={percent === null ? undefined : `${percent}%`}
      >
        <span
          className="progress-fill"
          style={percent === null ? undefined : { width: `${percent}%` }}
        />
      </div>
      {onCancel !== undefined && (
        <Button ref={cancelRef} variant="secondary" wide onClick={onCancel}>
          {cancelLabel}
        </Button>
      )}
    </div>
  )
}
