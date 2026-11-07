import type { Ref } from 'react'
import { Button } from './Button'

export interface ErrorMessageProps {
  /** Full error text; it wraps and is never clipped. */
  readonly message: string
  readonly guidance?: string
  /** Shown as a retry action when present. */
  readonly retry?: {
    readonly label: string
    readonly onRetry: () => void
    readonly disabled?: boolean
    readonly ref?: Ref<HTMLButtonElement> | undefined
  }
  /** `alert` interrupts the user; omit it when the error is announced by a status region. */
  readonly announce?: boolean
}

/**
 * An error panel that keeps every word visible: long names and messages wrap via
 * `overflow-wrap`, and the retry action sits in the flow, so nothing is cut off at 320 px.
 */
export function ErrorMessage({ message, guidance, retry, announce = true }: ErrorMessageProps) {
  return (
    <div className="error-panel" role={announce ? 'alert' : undefined}>
      <p className="error-text">{message}</p>
      {guidance !== undefined && <p className="error-guidance">{guidance}</p>}
      {retry !== undefined && (
        <Button
          ref={retry.ref}
          variant="secondary"
          wide
          disabled={retry.disabled ?? false}
          onClick={retry.onRetry}
        >
          {retry.label}
        </Button>
      )}
    </div>
  )
}
