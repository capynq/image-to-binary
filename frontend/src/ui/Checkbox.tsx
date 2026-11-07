export interface CheckboxProps {
  /** Form field name. */
  readonly name: string
  /** Visible label, which is also the accessible name. */
  readonly label: string
  readonly checked: boolean
  /** Receives the new checked state after each change. */
  readonly onChange: (checked: boolean) => void
}

/** A native checkbox wrapped in its label; the checked state is also drawn by the browser's own mark. */
export function Checkbox({ name, label, checked, onChange }: CheckboxProps) {
  return (
    <label className="check-row">
      <input
        type="checkbox"
        name={name}
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>{label}</span>
    </label>
  )
}
