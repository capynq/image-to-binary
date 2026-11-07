import { useId } from 'react'

export interface TextFieldProps {
  readonly id: string
  readonly name: string
  readonly label: string
  readonly value: string
  readonly onChange: (value: string) => void
  /** Full, unclipped problem text, or `null` when the value is acceptable. */
  readonly error: string | null
  /** Existing ids that describe the field, such as a visible note; the error id is added after them. */
  readonly describedBy?: string | undefined
}

/**
 * A text input with an error region. `aria-invalid` and `aria-describedby` follow the error, and
 * the error paragraph is a polite live region. The region is keyed only by its own text, so typing
 * that keeps the same error does not replace it and does not announce again.
 */
export function TextField({
  id,
  name,
  label,
  value,
  onChange,
  error,
  describedBy,
}: TextFieldProps) {
  const errorId = useId()
  const ids = [describedBy, error === null ? undefined : errorId].filter(
    (part): part is string => part !== undefined && part !== '',
  )
  return (
    <>
      <label className="field-label" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        className="text-input"
        name={name}
        type="text"
        value={value}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={error !== null}
        aria-describedby={ids.length === 0 ? undefined : ids.join(' ')}
        onChange={(event) => onChange(event.target.value)}
      />
      <p className="field-error" id={errorId} aria-live="polite">
        {error ?? ''}
      </p>
    </>
  )
}
