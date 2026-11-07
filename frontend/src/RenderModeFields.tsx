import { useId } from 'react'
import { validateRamp } from './processing/protocol'
import type { RenderModeName } from './processing/protocol'
import { SegmentedChoice, TextField } from './ui'

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
 * Render mode choice and the custom ramp field. The choice is a native radio group with a
 * description per mode; the ramp field reports its problem through `TextField`, which links the
 * error and the visible mode note with `aria-describedby`.
 */
export function RenderModeFields({
  mode,
  rampDraft,
  onModeChange,
  onRampDraftChange,
}: RenderModeFieldsProps) {
  const helpId = useId()
  const rampId = useId()
  const problem = rampProblem(mode, rampDraft)
  return (
    <>
      <SegmentedChoice
        legend="Render mode"
        options={MODE_OPTIONS.map((option) => ({
          ...option,
          description: MODE_HELP[option.value],
        }))}
        value={mode}
        onChange={onModeChange}
      />
      <p className="field-note" id={helpId}>
        {MODE_HELP[mode]}
      </p>
      {mode === 'ascii' && (
        <div className="ramp-field">
          <TextField
            id={rampId}
            name="ramp"
            label="Ramp, least ink first"
            value={rampDraft}
            onChange={onRampDraftChange}
            error={problem === null ? null : `Not applied: ${problem}.`}
            describedBy={helpId}
          />
        </div>
      )}
    </>
  )
}
