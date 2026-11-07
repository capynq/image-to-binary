import type { ExportFormat } from '../processing/types'
import { formatBytes } from '../useImportFlow'
import { EXPORT_FORMAT_WORDS } from './exportFormats'
import type { ExportPhase } from './exportFocus'

export interface AnnouncedExport {
  readonly phase: ExportPhase
  readonly text: string
}

/**
 * Text of the export live region after the phase changes from `previous` (null on mount), for the
 * export of `format`. Preparing keeps the running text so the internal download step is not spoken
 * as its own event. Only a running-to-idle transition is a cancellation; idle from any other phase
 * clears the text.
 */
export function announceExportPhase(
  previous: AnnouncedExport | null,
  phase: ExportPhase,
  byteLength: number | null,
  error: string | null,
  savedName: string | null = null,
  format: ExportFormat = 'png',
): string {
  const words = EXPORT_FORMAT_WORDS[format]
  switch (phase) {
    case 'running':
      return `Rendering ${words.name} export.`
    case 'preparing':
      return previous?.phase === 'running' ? previous.text : `Preparing ${words.name} download.`
    case 'ready':
      if (savedName !== null) return `Saved ${savedName}, ${formatBytes(byteLength ?? 0)}.`
      return `${words.lead} export ready, ${formatBytes(byteLength ?? 0)}.`
    case 'failed':
      return error === null
        ? `${words.lead} export failed.`
        : `${words.lead} export failed. ${error}`
    case 'idle':
      return previous?.phase === 'running' ? `${words.lead} export cancelled.` : ''
  }
}
