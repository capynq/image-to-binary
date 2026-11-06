import { formatBytes } from '../useImportFlow'
import type { ExportPhase } from './exportFocus'

export interface AnnouncedExport {
  readonly phase: ExportPhase
  readonly text: string
}

/**
 * Text of the export live region after the phase changes from `previous` (null on mount).
 * Preparing keeps the running text so the internal download step is not spoken as its own event.
 * Only a running-to-idle transition is a cancellation; idle from any other phase clears the text.
 */
export function announceExportPhase(
  previous: AnnouncedExport | null,
  phase: ExportPhase,
  byteLength: number | null,
  error: string | null,
): string {
  switch (phase) {
    case 'running':
      return 'Rendering PNG export.'
    case 'preparing':
      return previous?.phase === 'running' ? previous.text : 'Preparing PNG download.'
    case 'ready':
      return `PNG export ready, ${formatBytes(byteLength ?? 0)}.`
    case 'failed':
      return error === null ? 'PNG export failed.' : `PNG export failed. ${error}`
    case 'idle':
      return previous?.phase === 'running' ? 'PNG export cancelled.' : ''
  }
}
