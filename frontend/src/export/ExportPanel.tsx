import { useEffect, useId, useRef, useState } from 'react'
import { focusReachable } from '../studioActions'
import {
  Button,
  ErrorMessage,
  InspectorSection,
  Progress,
  SegmentedChoice,
  StatusRegion,
} from '../ui'
import type { ExportFormat } from '../processing/types'
import { formatBytes } from '../useImportFlow'
import { announceExportPhase } from './exportAnnouncement'
import type { AnnouncedExport } from './exportAnnouncement'
import type { DownloadLink } from './downloadOwner'
import { EXPORT_FORMAT_WORDS, exportFormatChoices } from './exportFormats'
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
  /** Formats the choice control offers; the owner passes GIF only for an animated source. */
  readonly formats?: readonly ExportFormat[]
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
  const reasonId = useId()
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

  const download = phase === 'ready' ? props.download : null
  const shownWords = EXPORT_FORMAT_WORDS[shown]
  const choiceWords = EXPORT_FORMAT_WORDS[choice]
  const formatChoice = props.onFormatChange
  const showsFormatChoice = phase !== 'running' && phase !== 'preparing'

  return (
    <InspectorSection
      title="Export"
      className="export-section"
      sectionRef={panelRef}
      headingRef={headingRef}
      focusableHeading
    >
      <p className="section-help">{choiceWords.help}</p>

      {formatChoice !== undefined && showsFormatChoice && (
        <SegmentedChoice
          legend="Format"
          columns={2}
          className="format-fieldset"
          optionsClassName="format-options"
          options={(props.formats ?? exportFormatChoices(false)).map((value) => ({
            value,
            label: EXPORT_FORMAT_WORDS[value].choice,
          }))}
          value={choice}
          onChange={formatChoice}
        />
      )}

      {phase === 'idle' && (
        <div className="export-actions">
          <Button
            ref={exportRef}
            variant="primary"
            wide
            disabled={!canExport}
            aria-describedby={disabledReason === null ? undefined : reasonId}
            onClick={props.onExport}
          >
            Export {choiceWords.name}
          </Button>
          {disabledReason !== null && (
            <p className="section-help export-reason" id={reasonId}>
              {disabledReason}
            </p>
          )}
        </div>
      )}

      {(phase === 'running' || phase === 'preparing') && (
        <div className="export-actions">
          <Progress
            label={phase === 'running' ? `Rendering ${shownWords.name}` : 'Preparing download'}
            fraction={phase === 'preparing' ? 1 : progress}
            onCancel={phase === 'running' ? props.onCancel : undefined}
            cancelLabel="Cancel export"
            cancelRef={cancelRef}
          />
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
          <Button
            ref={againRef}
            variant="secondary"
            wide
            disabled={!canExport}
            onClick={props.onExport}
          >
            Export again
          </Button>
        </div>
      )}

      {phase === 'failed' && (
        <div className="export-error">
          <ErrorMessage
            announce={false}
            message={error ?? `The ${shownWords.name} export failed.`}
            retry={{
              label: `Retry ${choiceWords.name} export`,
              onRetry: props.onExport,
              disabled: !canExport,
              ref: retryRef,
            }}
          />
        </div>
      )}

      <StatusRegion className="section-help export-status">{live.text}</StatusRegion>
    </InspectorSection>
  )
}
