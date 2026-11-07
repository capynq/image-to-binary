import { useId } from 'react'
import type { ChangeEvent } from 'react'
import { MAX_CONTRAST, MIN_CONTRAST } from './processing/bounds'
import { validateColour } from './processing/protocol'
import type { ColorModeName, ColourRole } from './processing/protocol'
import type { StudioSettings } from './processing/types'

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

interface ColourFieldProps {
  readonly role: ColourRole
  readonly accepted: string
  readonly draft: string
  readonly onColourChange: (role: ColourRole, value: string) => void
  readonly onColourDraftChange: (role: ColourRole, draft: string) => void
}

/**
 * A native colour picker, which is always valid, beside a hex text field. The text field keeps
 * the typed draft and forwards only valid hex; an invalid draft is explained in a polite live
 * region and linked with `aria-describedby`.
 */
function ColourField({
  role,
  accepted,
  draft,
  onColourChange,
  onColourDraftChange,
}: ColourFieldProps) {
  const pickerId = useId()
  const hexId = useId()
  const helpId = useId()
  const errorId = useId()
  const problem = colourProblem(draft, role)
  const title = COLOUR_TITLE[role]

  function handleHexChange(event: ChangeEvent<HTMLInputElement>) {
    const next = event.target.value
    onColourDraftChange(role, next)
    if (colourProblem(next, role) === null) onColourChange(role, next)
  }

  return (
    <div className="colour-field">
      <div className="colour-controls">
        <div className="colour-picker">
          <label className="field-label" htmlFor={pickerId}>
            {title}
          </label>
          <input
            id={pickerId}
            className="colour-input"
            type="color"
            name={`${role}Picker`}
            value={accepted}
            onClick={() => onColourDraftChange(role, accepted)}
            onChange={(event) => onColourChange(role, event.target.value)}
          />
        </div>
        <div className="colour-hex">
          <label className="field-label" htmlFor={hexId}>
            {title} hex
          </label>
          <input
            id={hexId}
            className="text-input"
            name={role}
            type="text"
            value={draft}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={problem !== null}
            aria-describedby={`${helpId} ${errorId}`}
            onChange={handleHexChange}
          />
        </div>
      </div>
      <p className="field-note" id={helpId}>
        Enter a colour as #RRGGBB, for example #00ff00.
      </p>
      <p className="field-error" id={errorId} aria-live="polite">
        {problem === null ? '' : `Not applied: ${problem}.`}
      </p>
    </div>
  )
}

/**
 * Colour, colour mode, contrast and invert controls. Every control is a native element, so the
 * keyboard operates them without custom handlers; contrast and invert apply on each change.
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
  const modeGroupName = useId()
  const contrastId = useId()
  return (
    <div className="style-fields">
      <ColourField
        role="foreground"
        accepted={settings.foreground}
        draft={foregroundDraft}
        onColourChange={onColourChange}
        onColourDraftChange={onColourDraftChange}
      />
      <ColourField
        role="background"
        accepted={settings.background}
        draft={backgroundDraft}
        onColourChange={onColourChange}
        onColourDraftChange={onColourDraftChange}
      />
      <fieldset className="mode-fieldset">
        <legend className="field-label-legend">Colour mode</legend>
        <div className="choice-options">
          {COLOR_MODE_OPTIONS.map((option) => (
            <label
              key={option.value}
              className={`mode-option ${settings.colorMode === option.value ? 'is-selected' : ''}`}
            >
              <input
                type="radio"
                name={modeGroupName}
                value={option.value}
                checked={settings.colorMode === option.value}
                onChange={() => onColorModeChange(option.value)}
              />
              <span>{option.label}</span>
            </label>
          ))}
        </div>
      </fieldset>
      <div className="range-heading">
        <label className="field-label" htmlFor={contrastId}>
          Contrast
        </label>
        <output htmlFor={contrastId}>{settings.contrast}</output>
      </div>
      <input
        id={contrastId}
        name="contrast"
        autoComplete="off"
        type="range"
        min={MIN_CONTRAST}
        max={MAX_CONTRAST}
        step="1"
        value={settings.contrast}
        aria-valuetext={String(settings.contrast)}
        onChange={(event) => onContrastChange(Number(event.target.value))}
      />
      <label className="check-row">
        <input
          type="checkbox"
          name="invert"
          checked={settings.invert}
          onChange={(event) => onInvertChange(event.target.checked)}
        />
        <span>Invert tones</span>
      </label>
    </div>
  )
}
