export interface SliderFieldProps {
  readonly id: string
  readonly name: string
  /** Stable accessible name; it never includes the current value. */
  readonly label: string
  readonly min: number
  readonly max: number
  readonly step?: number
  readonly value: number
  /** Appended to the value in the visible output and in `aria-valuetext`, for example `px`. */
  readonly unit?: string
  readonly disabled?: boolean
  readonly describedBy?: string
  readonly onChange: (value: number) => void
}

/**
 * A range input whose label and value output sit side by side, never nested: the label names the
 * control and the `output` shows its value, linked with `htmlFor`. `aria-valuetext` repeats the
 * value with its unit so assistive technology does not read a bare number.
 */
export function SliderField({
  id,
  name,
  label,
  min,
  max,
  step,
  value,
  unit = '',
  disabled = false,
  describedBy,
  onChange,
}: SliderFieldProps) {
  const shown = `${value}${unit}`
  return (
    <>
      <div className="range-heading">
        <label className="field-label" htmlFor={id}>
          {label}
        </label>
        <output htmlFor={id}>{shown}</output>
      </div>
      <input
        id={id}
        name={name}
        autoComplete="off"
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        aria-valuetext={shown}
        aria-describedby={describedBy}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </>
  )
}
