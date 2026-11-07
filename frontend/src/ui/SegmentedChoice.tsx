import { useId } from 'react'

export interface SegmentedOption<T extends string> {
  readonly value: T
  readonly label: string
  /** Linked to the option's radio with `aria-describedby`; visible help belongs to the caller. */
  readonly description?: string | undefined
  readonly disabled?: boolean
}

export interface SegmentedChoiceProps<T extends string> {
  readonly legend: string
  /** Hides the legend visually while keeping it as the group's accessible name. */
  readonly legendHidden?: boolean
  readonly options: readonly SegmentedOption<T>[]
  readonly value: T
  readonly onChange: (value: T) => void
  readonly columns?: 2 | 3
  readonly className?: string
  /** Extra class on the options wrapper, kept for stylesheet and harness hooks. */
  readonly optionsClassName?: string
}

/**
 * A native radio group inside a fieldset. The selected option carries two non-colour cues: a
 * check mark drawn from `data-state="selected"` and a heavier border, so the choice is never
 * conveyed by colour alone. Arrow keys move the choice because the inputs are native radios.
 */
export function SegmentedChoice<T extends string>({
  legend,
  legendHidden = false,
  options,
  value,
  onChange,
  columns = 3,
  className,
  optionsClassName,
}: SegmentedChoiceProps<T>) {
  const groupName = useId()
  return (
    <fieldset className={`mode-fieldset segmented-choice ${className ?? ''}`.trim()}>
      <legend className={legendHidden ? 'visually-hidden' : 'field-label-legend'}>{legend}</legend>
      <div className={`mode-options segmented-columns-${columns} ${optionsClassName ?? ''}`.trim()}>
        {options.map((option, index) => {
          const selected = option.value === value
          return (
            <label
              key={option.value}
              className={`mode-option ${selected ? 'is-selected' : ''} ${option.disabled === true ? 'is-disabled' : ''}`.trim()}
            >
              <input
                type="radio"
                name={groupName}
                value={option.value}
                checked={selected}
                disabled={option.disabled ?? false}
                aria-describedby={
                  option.description === undefined ? undefined : `${groupName}-${index}`
                }
                onChange={() => onChange(option.value)}
              />
              <span>{option.label}</span>
              <span
                className="choice-cue"
                aria-hidden="true"
                data-state={selected ? 'selected' : 'unselected'}
              >
                {selected && (
                  <svg className="choice-check" viewBox="0 0 12 12" focusable="false">
                    <path
                      d="M2.2 6.4 4.8 9l5-6"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                )}
              </span>
            </label>
          )
        })}
      </div>
      {options.map((option, index) =>
        option.description === undefined ? null : (
          <span key={option.value} className="visually-hidden" id={`${groupName}-${index}`}>
            {option.description}
          </span>
        ),
      )}
    </fieldset>
  )
}
