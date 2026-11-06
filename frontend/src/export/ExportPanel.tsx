import { useEffect, useId, useRef, useState } from 'react'
import { formatBytes } from '../useImportFlow'
import { announceExportPhase } from './exportAnnouncement'
import type { AnnouncedExport } from './exportAnnouncement'
import type { DownloadLink } from './downloadOwner'
import { exportFocusTarget, shouldMoveExportFocus } from './exportFocus'
import type { ExportFocusTarget, ExportPhase } from './exportFocus'

export interface ExportPanelProps {
  readonly phase: ExportPhase
  readonly canExport: boolean
  readonly disabledReason: string | null
  /** Fraction in [0, 1] of the running encode. */
  readonly progress: number
  readonly byteLength: number | null
  readonly error: string | null
  readonly download: DownloadLink | null
  readonly onExport: () => void
  readonly onCancel: () => void
}

/**
 * Inspector export section. Every phase renders one primary control: Export, Cancel, Download
 * (with Export again), or Retry. Focus follows `exportFocusTarget` when the control it was on is
 * replaced, so keyboard users are never left on `body`. This panel's live region is the only
 * announcer of the export lifecycle.
 */
export function ExportPanel(props: ExportPanelProps) {
  const { phase, canExport, disabledReason, progress, byteLength, error } = props
  const [announced, setAnnounced] = useState<AnnouncedExport>(() => ({
    phase,
    text: announceExportPhase(null, phase, byteLength, error),
  }))
  const headingId = useId()
  const reasonId = useId()
  const progressId = useId()
  const panelRef = useRef<HTMLElement>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)
  const exportRef = useRef<HTMLButtonElement>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)
  const downloadRef = useRef<HTMLAnchorElement>(null)
  const retryRef = useRef<HTMLButtonElement>(null)
  const previousPhase = useRef<ExportPhase>(phase)

  useEffect(() => {
    const from = previousPhase.current
    previousPhase.current = phase
    if (from === phase) return
    const target = exportFocusTarget(phase, canExport)
    if (target === null) return
    const active = document.activeElement
    const onBody = active === null || active === document.body
    const insidePanel =
      panelRef.current !== null && active !== null && panelRef.current.contains(active)
    if (!shouldMoveExportFocus(onBody, insidePanel)) return
    const elements: Record<ExportFocusTarget, HTMLElement | null> = {
      export: exportRef.current,
      cancel: cancelRef.current,
      download: downloadRef.current,
      retry: retryRef.current,
      heading: headingRef.current,
    }
    elements[target]?.focus()
  }, [phase, canExport])

  let live = announced
  if (announced.phase !== phase) {
    live = { phase, text: announceExportPhase(announced, phase, byteLength, error) }
    setAnnounced(live)
  }

  const percent = Math.round(Math.min(1, Math.max(0, progress)) * 100)
  const download = phase === 'ready' ? props.download : null

  return (
    <section className="control-section export-section" aria-labelledby={headingId} ref={panelRef}>
      <div className="section-heading">
        <h2 id={headingId} ref={headingRef} tabIndex={-1}>
          Export
        </h2>
      </div>
      <p className="section-help">
        PNG uses the current rendering settings and stays on this device.
      </p>

      {phase === 'idle' && (
        <div className="export-actions">
          <button
            ref={exportRef}
            className="button button-primary button-wide"
            type="button"
            disabled={!canExport}
            aria-describedby={disabledReason === null ? undefined : reasonId}
            onClick={props.onExport}
          >
            Export PNG
          </button>
          {disabledReason !== null && (
            <p className="section-help export-reason" id={reasonId}>
              {disabledReason}
            </p>
          )}
        </div>
      )}

      {(phase === 'running' || phase === 'preparing') && (
        <div className="export-actions">
          <div className="export-progress-head">
            <label htmlFor={progressId}>
              {phase === 'running' ? 'Rendering PNG' : 'Preparing download'}
            </label>
            <span className="export-percent">{phase === 'preparing' ? 100 : percent}%</span>
          </div>
          <progress
            id={progressId}
            className="export-progress"
            max={100}
            value={phase === 'preparing' ? 100 : percent}
          />
          {phase === 'running' && (
            <button
              ref={cancelRef}
              className="button button-secondary button-wide"
              type="button"
              onClick={props.onCancel}
            >
              Cancel export
            </button>
          )}
        </div>
      )}

      {phase === 'ready' && (
        <div className="export-actions">
          <p className="section-help">Encoded PNG: {formatBytes(byteLength ?? 0)}</p>
          {download !== null && (
            <a
              ref={downloadRef}
              className="button button-primary button-wide"
              href={download.url}
              download={download.filename}
            >
              Download PNG
            </a>
          )}
          <button
            className="button button-secondary button-wide"
            type="button"
            disabled={!canExport}
            onClick={props.onExport}
          >
            Export again
          </button>
        </div>
      )}

      {phase === 'failed' && (
        <div className="error-panel export-error">
          <p className="error-text">{error ?? 'The PNG export failed.'}</p>
          <button
            ref={retryRef}
            className="button button-secondary button-wide"
            type="button"
            disabled={!canExport}
            onClick={props.onExport}
          >
            Retry PNG export
          </button>
        </div>
      )}

      <p className="section-help export-status" role="status" aria-live="polite">
        {live.text}
      </p>
    </section>
  )
}
