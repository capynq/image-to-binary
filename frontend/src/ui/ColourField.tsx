import { useId } from 'react'
import { TextField } from './TextField'

export interface ColourFieldProps {
  readonly label: string
  readonly pickerName: string
  readonly hexName: string
  /** The colour the render uses; the picker always shows it. */
  readonly accepted: string
  /** The hex text as typed. */
  readonly draft: string
  /** Problem text for the draft, or `null` when it is a valid colour. */
  readonly error: string | null
  readonly onPick: (value: string) => void
  readonly onDraftChange: (draft: string) => void
  /** Called when the picker opens; the caller resets the draft to the accepted colour. */
  readonly onPickerOpen?: () => void
}

/**
 * A native colour picker beside a hex text field. The picker is always valid and applies at once;
 * the hex field reports the caller's problem text through `TextField`.
 */
export function ColourField({
  label,
  pickerName,
  hexName,
  accepted,
  draft,
  error,
  onPick,
  onDraftChange,
  onPickerOpen,
}: ColourFieldProps) {
  const pickerId = useId()
  const hexId = useId()
  const helpId = useId()

  return (
    <div className="colour-field">
      <div className="colour-controls">
        <div className="colour-picker">
          <label className="field-label" htmlFor={pickerId}>
            {label}
          </label>
          <input
            id={pickerId}
            className="colour-input"
            type="color"
            name={pickerName}
            value={accepted}
            onClick={onPickerOpen}
            onChange={(event) => onPick(event.target.value)}
          />
        </div>
        <div className="colour-hex">
          <TextField
            id={hexId}
            name={hexName}
            label={`${label} hex`}
            value={draft}
            onChange={onDraftChange}
            error={error}
            describedBy={helpId}
          />
        </div>
      </div>
      <p className="field-note" id={helpId}>
        Enter a colour as #RRGGBB, for example #00ff00.
      </p>
    </div>
  )
}
