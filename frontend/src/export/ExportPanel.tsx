import { useEffect, useId, useRef, useState } from 'react'
import { focusReachable } from '../studioActions'
import type { ExportFormat } from '../processing/types'
import { formatBytes } from '../useImportFlow'
import { announceExportPhase } from './exportAnnouncement'
import type { AnnouncedExport } from './exportAnnouncement'
import type { DownloadLink } from './downloadOwner'
import { EXPORT_FORMAT_WORDS, EXPORT_FORMATS } from './exportFormats'
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
  /** Name of a native export already written to the chosen file; replaces the download link. */
  readonly savedName?: string | null
  /** The format the next export produces. The format control is shown only when this is set. */
  readonly format?: ExportFormat
  readonly onFormatChange?: (format: ExportFormat) => void
  /** The format of the export on screen, such as the running or downloaded one; defaults to `format`. */
  readonly shownFormat?: ExportFormat
  readonly onExport: () => void
  readonly onCancel: () => void
  /**
   * Bumped by the owner each time an export settles, whatever the outcome. A change moves focus
   * to the phase's control when focus was lost, so a cancelled save never leaves it on body.
   */
  readonly focusRequest?: number
}

/**
 * Inspector export section. Every phase renders one primary control: Export, Cancel, Download
 * (with Export again), or Retry. Focus follows `exportFocusTarget` when the control it was on is
 * replaced, so keyboard users are never left on `body`. This panel's live region is the only
 * announcer of the export lifecycle.
 *
 * The format control is a radio group, not a second export button: one primary control, one
 * focus path and one announcement path stay for every format. It is hidden while an export runs,
 * so the format of a running export cannot change under it.
 */
export function ExportPanel(props: ExportPanelProps) {
  const { phase, canExport, disabledReason, progress, byteLength, error } = props
  const savedName = props.savedName ?? null
  const choice: ExportFormat = props.format ?? 'png'
  const shown: ExportFormat = props.shownFormat ?? choice
  const [announced, setAnnounced] = useState<AnnouncedExport>(() => ({
    phase,
    text: announceExportPhase(null, phase, byteLength, error, savedName, shown),
  }))
  const headingId = useId()
  const reasonId = useId()
  const formatGroupId = useId()
  const progressId = useId()
  const panelRef = useRef<HTMLElement>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)
  const exportRef = useRef<HTMLButtonElement>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)
  const downloadRef = useRef<HTMLAnchorElement>(null)
  const retryRef = useRef<HTMLButtonElement>(null)
  const againRef = useRef<HTMLButtonElement>(null)
  const previousPhase = useRef<ExportPhase>(phase)
  const lastFocusRequest = useRef(props.focusRequest)

  useEffect(() => {
    const from = previousPhase.current
    previousPhase.current = phase
    if (from === phase) return
    const target = exportFocusTarget(phase, canExport, savedName !== null)
    if (target === null) return
    const active = document.activeElement
    const onBody = active === null || active === document.body
    const insidePanel =
      panelRef.current !== null && active !== null && panelRef.current.contains(active)
    if (!shouldMoveExportFocus(onBody, insidePanel)) return
    focusExportTarget(target)
  }, [phase, canExport, savedName])

  useEffect(() => {
    if (props.focusRequest === lastFocusRequest.current) return
    lastFocusRequest.current = props.focusRequest
    const target = exportFocusTarget(phase, canExport, savedName !== null)
    if (target === null) return
    const active = document.activeElement
    const onBody = active === null || active === document.body
    const insidePanel =
      panelRef.current !== null && active !== null && panelRef.current.contains(active)
    if (!shouldMoveExportFocus(onBody, insidePanel)) return
    focusExportTarget(target)
  }, [props.focusRequest, phase, canExport, savedName])

  function focusExportTarget(target: ExportFocusTarget): void {
    const elements: Record<ExportFocusTarget, HTMLElement | null> = {
      export: exportRef.current,
      cancel: cancelRef.current,
      download: downloadRef.current,
      again: againRef.current,
      retry: retryRef.current,
      heading: headingRef.current,
    }
    focusReachable(elements[target], headingRef.current)
  }

  let live = announced
  if (announced.phase !== phase) {
    live = {
      phase,
      text: announceExportPhase(announced, phase, byteLength, error, savedName, shown),
    }
    setAnnounced(live)
  }

  const percent = Math.round(Math.min(1, Math.max(0, progress)) * 100)
  const download = phase === 'ready' ? props.download : null
  const shownWords = EXPORT_FORMAT_WORDS[shown]
  const choiceWords = EXPORT_FORMAT_WORDS[choice]
  const formatChoice = props.onFormatChange
  const showsFormatChoice = phase !== 'running' && phase !== 'preparing'

  return (
    <section className="control-section export-section" aria-labelledby={headingId} ref={panelRef}>
      <div className="section-heading">
        <h2 id={headingId} ref={headingRef} tabIndex={-1}>
          Export
        </h2>
      </div>
      <p className="section-help">{choiceWords.help}</p>

      {formatChoice !== undefined && showsFormatChoice && (
        <fieldset className="mode-fieldset format-fieldset">
          <legend className="field-label-legend">Format</legend>
          <div className="mode-options format-options">
            {EXPORT_FORMATS.map((value) => (
              <label key={value} className={`mode-option ${choice === value ? 'is-selected' : ''}`}>
                <input
                  type="radio"
                  name={formatGroupId}
                  value={value}
                  checked={choice === value}
                  onChange={() => formatChoice(value)}
                />
                <span>{EXPORT_FORMAT_WORDS[value].choice}</span>
              </label>
            ))}
          </div>
        </fieldset>
      )}

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
            Export {choiceWords.name}
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
              {phase === 'running' ? `Rendering ${shownWords.name}` : 'Preparing download'}
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
          {savedName !== null ? (
            <p className="section-help">
              Saved {savedName}, {formatBytes(byteLength ?? 0)}.
            </p>
          ) : (
            <p className="section-help">
              Encoded {shownWords.name}: {formatBytes(byteLength ?? 0)}
            </p>
          )}
          {download !== null && (
            <a
              ref={downloadRef}
              className="button button-primary button-wide"
              href={download.url}
              download={download.filename}
            >
              Download {shownWords.name}
            </a>
          )}
          <button
            ref={againRef}
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
          <p className="error-text">{error ?? `The ${shownWords.name} export failed.`}</p>
          <button
            ref={retryRef}
            className="button button-secondary button-wide"
            type="button"
            disabled={!canExport}
            onClick={props.onExport}
          >
            Retry {choiceWords.name} export
          </button>
        </div>
      )}

      <p className="section-help export-status" role="status" aria-live="polite">
        {live.text}
      </p>
    </section>
  )
}
