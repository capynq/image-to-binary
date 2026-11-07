import { useId } from 'react'
import { MAX_CONTRAST, MIN_CONTRAST } from './processing/bounds'
import { validateColour } from './processing/protocol'
import type { ColorModeName, ColourRole } from './processing/protocol'
import type { StudioSettings } from './processing/types'
import { Checkbox, ColourField, SegmentedChoice, SliderField } from './ui'

export interface RenderStyleFieldsProps {
  readonly settings: StudioSettings
  /** The foreground text as typed; it is applied only while it is a valid `#RRGGBB` colour. */
  readonly foregroundDraft: string
  /** The background text as typed; it is applied only while it is a valid `#RRGGBB` colour. */
  readonly backgroundDraft: string
  readonly onColourChange: (role: ColourRole, value: string) => void
  readonly onColourDraftChange: (role: ColourRole, draft: string) => void
  readonly onColorModeChange: (mode: ColorModeName) => void
  readonly onContrastChange: (value: number) => void
  readonly onInvertChange: (value: boolean) => void
}

const COLOUR_TITLE: Readonly<Record<ColourRole, string>> = {
  foreground: 'Foreground colour',
  background: 'Background colour',
}

const COLOR_MODE_OPTIONS: ReadonlyArray<{ readonly value: ColorModeName; readonly label: string }> =
  [
    { value: 'monochrome', label: 'Monochrome' },
    { value: 'source', label: 'Source colour' },
  ]

/** The colour problem that blocks preview and export, or `null`. */
export function colourProblem(draft: string, role: ColourRole): string | null {
  return validateColour(draft, role)
}

interface StyleColourFieldProps {
  readonly role: ColourRole
  readonly accepted: string
  readonly draft: string
  readonly onColourChange: (role: ColourRole, value: string) => void
  readonly onColourDraftChange: (role: ColourRole, draft: string) => void
}

/** Connects a `ColourField` to the draft rule: only a valid hex reaches the accepted colour. */
function StyleColourField({
  role,
  accepted,
  draft,
  onColourChange,
  onColourDraftChange,
}: StyleColourFieldProps) {
  const problem = colourProblem(draft, role)
  return (
    <ColourField
      label={COLOUR_TITLE[role]}
      pickerName={`${role}Picker`}
      hexName={role}
      accepted={accepted}
      draft={draft}
      error={problem === null ? null : `Not applied: ${problem}.`}
      onPick={(value) => onColourChange(role, value)}
      onDraftChange={(next) => {
        onColourDraftChange(role, next)
        if (colourProblem(next, role) === null) onColourChange(role, next)
      }}
      onPickerOpen={() => onColourDraftChange(role, accepted)}
    />
  )
}

/**
 * Colour, colour mode, contrast and invert controls. Every control is a native element; the
 * contrast slider and invert checkbox apply on each change.
 */
export function RenderStyleFields({
  settings,
  foregroundDraft,
  backgroundDraft,
  onColourChange,
  onColourDraftChange,
  onColorModeChange,
  onContrastChange,
  onInvertChange,
}: RenderStyleFieldsProps) {
  const contrastId = useId()
  return (
    <div className="style-fields">
      <StyleColourField
        role="foreground"
        accepted={settings.foreground}
        draft={foregroundDraft}
        onColourChange={onColourChange}
        onColourDraftChange={onColourDraftChange}
      />
      <StyleColourField
        role="background"
        accepted={settings.background}
        draft={backgroundDraft}
        onColourChange={onColourChange}
        onColourDraftChange={onColourDraftChange}
      />
      <SegmentedChoice
        legend="Colour mode"
        columns={2}
        options={COLOR_MODE_OPTIONS}
        value={settings.colorMode}
        onChange={onColorModeChange}
      />
      <SliderField
        id={contrastId}
        name="contrast"
        label="Contrast"
        min={MIN_CONTRAST}
        max={MAX_CONTRAST}
        step={1}
        value={settings.contrast}
        onChange={onContrastChange}
      />
      <Checkbox
        name="invert"
        label="Invert tones"
        checked={settings.invert}
        onChange={onInvertChange}
      />
    </div>
  )
}
