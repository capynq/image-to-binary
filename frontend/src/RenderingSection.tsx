import {
  MAX_CELL_SIZE,
  MAX_FONT_SIZE,
  MAX_THRESHOLD,
  MIN_CELL_SIZE,
  MIN_FONT_SIZE,
  MIN_THRESHOLD,
} from './processing/bounds'
import type { ColorModeName, ColourRole, RenderModeName } from './processing/protocol'
import type { StudioSettings } from './processing/types'
import { RenderModeFields } from './RenderModeFields'
import { RenderStyleFields } from './RenderStyleFields'
import { Button, InspectorSection, SliderField } from './ui'

export type SizeSetting = 'cellSize' | 'fontSize' | 'threshold'

export interface RenderingSectionProps {
  readonly settings: StudioSettings
  readonly rampDraft: string
  readonly foregroundDraft: string
  readonly backgroundDraft: string
  readonly thresholdApplies: boolean
  readonly renderEnabled: boolean
  readonly rendering: boolean
  readonly onSizeChange: (key: SizeSetting, value: number) => void
  readonly onModeChange: (mode: RenderModeName) => void
  readonly onRampDraftChange: (draft: string) => void
  readonly onColourChange: (role: ColourRole, value: string) => void
  readonly onColourDraftChange: (role: ColourRole, draft: string) => void
  readonly onColorModeChange: (mode: ColorModeName) => void
  readonly onContrastChange: (value: number) => void
  readonly onInvertChange: (value: boolean) => void
  readonly onRender: () => void
}

/** Inspector rendering controls: size, mode, threshold, colour and tone, then the render action. */
export function RenderingSection({
  settings,
  rampDraft,
  foregroundDraft,
  backgroundDraft,
  thresholdApplies,
  renderEnabled,
  rendering,
  onSizeChange,
  onModeChange,
  onRampDraftChange,
  onColourChange,
  onColourDraftChange,
  onColorModeChange,
  onContrastChange,
  onInvertChange,
  onRender,
}: RenderingSectionProps) {
  return (
    <InspectorSection title="Rendering">
      <SliderField
        id="cell-size"
        name="cellSize"
        label="Cell size"
        unit="px"
        min={MIN_CELL_SIZE}
        max={MAX_CELL_SIZE}
        value={settings.cellSize}
        onChange={(value) => onSizeChange('cellSize', value)}
      />
      <SliderField
        id="font-size"
        name="fontSize"
        label="Font size"
        unit="px"
        min={MIN_FONT_SIZE}
        max={MAX_FONT_SIZE}
        value={settings.fontSize}
        onChange={(value) => onSizeChange('fontSize', value)}
      />
      <RenderModeFields
        mode={settings.mode}
        rampDraft={rampDraft}
        onModeChange={onModeChange}
        onRampDraftChange={onRampDraftChange}
      />
      <SliderField
        id="threshold"
        name="threshold"
        label="Threshold"
        min={MIN_THRESHOLD}
        max={MAX_THRESHOLD}
        value={settings.threshold}
        disabled={!thresholdApplies}
        describedBy="threshold-note"
        onChange={(value) => onSizeChange('threshold', value)}
      />
      <p className="field-note" id="threshold-note">
        {thresholdApplies
          ? 'The threshold splits cells into 0 and 1 in classic binary mode.'
          : 'The threshold has no effect in ASCII or block mode.'}
      </p>
      <RenderStyleFields
        settings={settings}
        foregroundDraft={foregroundDraft}
        backgroundDraft={backgroundDraft}
        onColourChange={onColourChange}
        onColourDraftChange={onColourDraftChange}
        onColorModeChange={onColorModeChange}
        onContrastChange={onContrastChange}
        onInvertChange={onInvertChange}
      />
      <p className="field-note" id="render-note">
        Updates automatically after changes.
      </p>
      <Button
        variant="primary"
        wide
        busy={rendering}
        disabled={!renderEnabled}
        aria-describedby="render-note"
        onClick={onRender}
      >
        {rendering ? 'Rendering…' : 'Render artwork'}
      </Button>
    </InspectorSection>
  )
}
