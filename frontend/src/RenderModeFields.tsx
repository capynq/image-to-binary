import { useId } from 'react'
import { validateRamp } from './processing/protocol'
import type { RenderModeName } from './processing/protocol'

export interface RenderModeFieldsProps {
  readonly mode: RenderModeName
  /** The ramp text as typed; it is applied only while it is valid. */
  readonly rampDraft: string
  readonly onModeChange: (mode: RenderModeName) => void
  readonly onRampDraftChange: (draft: string) => void
}

const MODE_OPTIONS: ReadonlyArray<{ readonly value: RenderModeName; readonly label: string }> = [
  { value: 'binary', label: 'Classic binary' },
  { value: 'ascii', label: 'ASCII' },
  { value: 'block', label: 'Block' },
]

const MODE_HELP: Readonly<Record<RenderModeName, string>> = {
  binary: 'Binary 0 / 1 output, split at the threshold.',
  ascii: 'ASCII uses average sampling. Brighter cells take later ramp characters.',
  block: 'Block uses average sampling and the Unicode block elements.',
}

/** The ramp problem that blocks preview and export, or `null`. Only ASCII reads the ramp. */
export function rampProblem(mode: RenderModeName, rampDraft: string): string | null {
  return mode === 'ascii' ? validateRamp(rampDraft) : null
}

/**
 * Render mode choice and the custom ramp field. Native radio buttons and a text input keep the
 * controls keyboard operable; the ramp error is linked with `aria-describedby` and announced
 * politely through its live region.
 */
export function RenderModeFields({
  mode,
  rampDraft,
  onModeChange,
  onRampDraftChange,
}: RenderModeFieldsProps) {
  const groupName = useId()
  const helpId = useId()
  const rampId = useId()
  const errorId = useId()
  const problem = rampProblem(mode, rampDraft)
  const describedBy = problem === null ? helpId : `${helpId} ${errorId}`
  return (
    <fieldset className="mode-fieldset">
      <legend className="field-label-legend">Render mode</legend>
      <div className="mode-options">
        {MODE_OPTIONS.map((option) => (
          <label
            key={option.value}
            className={`mode-option ${mode === option.value ? 'is-selected' : ''}`}
          >
            <input
              type="radio"
              name={groupName}
              value={option.value}
              checked={mode === option.value}
              aria-describedby={`${groupName}-${option.value}`}
              onChange={() => onModeChange(option.value)}
            />
            <span>{option.label}</span>
            <span className="visually-hidden" id={`${groupName}-${option.value}`}>
              {MODE_HELP[option.value]}
            </span>
          </label>
        ))}
      </div>
      <p className="field-note" id={helpId}>
        {MODE_HELP[mode]}
      </p>
      {mode === 'ascii' && (
        <div className="ramp-field">
          <label className="field-label" htmlFor={rampId}>
            Ramp, least ink first
          </label>
          <input
            id={rampId}
            className="text-input"
            name="ramp"
            type="text"
            value={rampDraft}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={problem !== null}
            aria-describedby={describedBy}
            onChange={(event) => onRampDraftChange(event.target.value)}
          />
          <p className="field-error" id={errorId} aria-live="polite">
            {problem === null ? '' : `Not applied: ${problem}.`}
          </p>
        </div>
      )}
    </fieldset>
  )
}
