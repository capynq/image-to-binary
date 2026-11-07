import { Progress, ErrorMessage } from './ui'

export interface StudioStatusProps {
  /** Import or preview work is running; the progress row is shown with a cancel action. */
  readonly busyLabel: string | null
  readonly onCancel: () => void
  readonly errorMessage: string | null
  readonly guidance: string
  readonly onChooseAnother: () => void
}

/** The canvas-level progress row and error panel, each with its visible text and actions. */
export function StudioStatus({
  busyLabel,
  onCancel,
  errorMessage,
  guidance,
  onChooseAnother,
}: StudioStatusProps) {
  return (
    <>
      {busyLabel !== null && (
        <div className="busy-row">
          <Progress label={busyLabel} onCancel={onCancel} />
        </div>
      )}
      {errorMessage !== null && (
        <ErrorMessage
          message={errorMessage}
          guidance={guidance}
          retry={{ label: 'Choose another image', onRetry: onChooseAnother }}
        />
      )}
    </>
  )
}
