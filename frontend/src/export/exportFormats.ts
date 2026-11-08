import type { ExportFormat } from '../processing/types'

export interface ExportFormatWords {
  /** Label of the format choice control. */
  readonly choice: string
  /** Name used mid-sentence in status and button text. */
  readonly name: string
  /** Name used at the start of a sentence. */
  readonly lead: string
  /** Help text of the export section while this format is chosen. */
  readonly help: string
}

/** Formats in the order the choice control lists them. */
export const EXPORT_FORMATS: readonly ExportFormat[] = ['png', 'text', 'gif']

/** The formats a source offers: GIF needs an animation, so a still gets PNG and text only. */
export function exportFormatChoices(animated: boolean): readonly ExportFormat[] {
  return animated ? EXPORT_FORMATS : EXPORT_FORMATS.filter((format) => format !== 'gif')
}

export const EXPORT_FORMAT_WORDS: Readonly<Record<ExportFormat, ExportFormatWords>> = Object.freeze(
  {
    png: {
      choice: 'PNG image',
      name: 'PNG',
      lead: 'PNG',
      help: 'PNG uses the current rendering settings and stays on this device.',
    },
    text: {
      choice: 'UTF-8 text',
      name: 'text',
      lead: 'Text',
      help: 'Text uses the same grid and settings as PNG and stays on this device.',
    },
    gif: {
      choice: 'Animated GIF',
      name: 'GIF',
      lead: 'GIF',
      help: 'GIF encodes every frame at the source timing with the current settings and stays on this device.',
    },
  },
)
