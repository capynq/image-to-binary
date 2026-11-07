export type ExportPhase = 'idle' | 'running' | 'preparing' | 'ready' | 'failed'

export type ExportFocusTarget = 'export' | 'cancel' | 'download' | 'again' | 'retry' | 'heading'

/**
 * Names the control that takes focus when the export phase becomes `to`. `null` means the
 * next phase renders no focusable control yet, so focus stays put until the following phase.
 * A saved export has no download link, so its ready phase focuses `Export again`.
 */
export function exportFocusTarget(
  to: ExportPhase,
  canExport: boolean,
  saved = false,
): ExportFocusTarget | null {
  switch (to) {
    case 'running':
      return 'cancel'
    case 'ready':
      return saved ? 'again' : 'download'
    case 'failed':
      return 'retry'
    case 'preparing':
      return null
    case 'idle':
      return canExport ? 'export' : 'heading'
  }
}

/**
 * Focus moves only when the previously focused control was removed (focus fell to the body) or
 * sat inside the export panel. Focus elsewhere, such as the header, is never taken away.
 */
export function shouldMoveExportFocus(focusOnBody: boolean, focusInsidePanel: boolean): boolean {
  return focusOnBody || focusInsidePanel
}
