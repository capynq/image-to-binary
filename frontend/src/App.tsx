import { useEffect, useId, useState } from 'react'
import { RenderModeFields, rampProblem } from './RenderModeFields'
import { RenderStyleFields, colourProblem } from './RenderStyleFields'
import { ExportPanel } from './export/ExportPanel'
import { exportFilename } from './export/downloadName'
import { EXPORT_FORMAT_WORDS } from './export/exportFormats'
import type { ExportPhase } from './export/exportFocus'
import { footerStatusLabel, useFooterAnnouncement } from './export/footerAnnouncement'
import { useExportDownload } from './export/useExportDownload'
import {
  MAX_CELL_SIZE,
  MAX_FONT_SIZE,
  MAX_THRESHOLD,
  MIN_CELL_SIZE,
  MIN_FONT_SIZE,
  MIN_THRESHOLD,
} from './processing/bounds'
import { defaultUrlApi } from './processing/demoAdapter'
import type { NativeFiles } from './processing/nativeFiles'
import type { ExportFormat, ExportStatus } from './processing/types'
import { validateRamp } from './processing/protocol'
import type { ColorModeName, ColourRole, RenderModeName } from './processing/protocol'
import type { StudioController } from './state/controller'
import { IMPORT_ACCEPT, IMPORT_GUIDANCE, formatBytes, useImportFlow } from './useImportFlow'
import { useStudio } from './useStudio'

type Theme = 'dark' | 'light'

const DOWNLOAD_URL_API = defaultUrlApi()

function getInitialTheme(): Theme {
  try {
    const storedTheme = window.localStorage.getItem('image-to-binary-theme')
    if (storedTheme === 'light' || storedTheme === 'dark') return storedTheme
  } catch {
    return 'dark'
  }
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

function exportPhaseOf(status: ExportStatus, hasDownload: boolean): ExportPhase {
  switch (status) {
    case 'running':
      return 'running'
    case 'failed':
      return 'failed'
    case 'ready':
      return hasDownload ? 'ready' : 'preparing'
    case 'idle':
      return 'idle'
  }
}

/** The draft that blocks preview and export; the ramp is named first when both are invalid. */
type DraftBlocker = 'ramp' | 'colour'

const DRAFT_BLOCK_NAME: Readonly<Record<DraftBlocker, string>> = {
  ramp: 'ramp',
  colour: 'colour',
}

const DRAFT_BLOCK_ARTWORK: Readonly<Record<DraftBlocker, string>> = {
  ramp: 'Fix the ramp to show the artwork.',
  colour: 'Fix the colour to show the artwork.',
}

function exportBlockedBy(
  status: string,
  hasSource: boolean,
  blocker: DraftBlocker | null,
  format: ExportFormat,
): string | null {
  const name = EXPORT_FORMAT_WORDS[format].name
  if (!hasSource) return `Open or load an image to enable ${name} export.`
  if (status === 'importing') return 'Wait for the image to finish importing.'
  if (status === 'processing') return 'Wait for the artwork preview to finish rendering.'
  if (blocker !== null) return `Fix the ${DRAFT_BLOCK_NAME[blocker]} to enable ${name} export.`
  return null
}

export interface AppProps {
  readonly controller: StudioController
  /** Present only inside the desktop shell, where dialogs and drops are native. */
  readonly native: NativeFiles | null
}

export default function App({ controller, native }: AppProps) {
  const { snapshot } = useStudio(controller)
  const flow = useImportFlow(controller, native)
  const hintId = useId()
  const [theme, setTheme] = useState<Theme>(getInitialTheme)
  const [inspectorOpen, setInspectorOpen] = useState(true)
  const [rampDraft, setRampDraft] = useState(() => snapshot.settings.ramp)
  const [foregroundDraft, setForegroundDraft] = useState(() => snapshot.settings.foreground)
  const [backgroundDraft, setBackgroundDraft] = useState(() => snapshot.settings.background)
  const [exportFormat, setExportFormat] = useState<ExportFormat>('png')
  const [runFormat, setRunFormat] = useState<ExportFormat>('png')
  const artworkReasonId = useId()

  useEffect(() => {
    document.documentElement.dataset.theme = theme
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute('content', theme === 'dark' ? '#0e1118' : '#f3f5fa')
    try {
      window.localStorage.setItem('image-to-binary-theme', theme)
    } catch {
      return
    }
  }, [theme])

  const hasSource = snapshot.metadata !== null
  const isBusy =
    snapshot.status === 'importing' ||
    snapshot.status === 'processing' ||
    snapshot.status === 'exporting'
  const settings = snapshot.settings

  useEffect(() => {
    setRampDraft(settings.ramp)
  }, [settings.ramp])

  useEffect(() => {
    setForegroundDraft(settings.foreground)
  }, [settings.foreground])

  useEffect(() => {
    setBackgroundDraft(settings.background)
  }, [settings.background])
  const isOriginal = flow.view === 'original'
  const shown = isOriginal ? snapshot.original : snapshot.preview
  const viewName = isOriginal ? 'original image' : 'artwork'
  const exportFailed = snapshot.export.status === 'failed'
  const errorMessage =
    flow.localError ?? (snapshot.status === 'error' && !exportFailed ? snapshot.error : null)
  const busyLabel =
    snapshot.status !== 'importing'
      ? 'Rendering preview…'
      : snapshot.importStage === 'heifDecoder'
        ? 'Loading HEIC/AVIF decoder…'
        : 'Importing image…'
  const readyExport = snapshot.export.status === 'ready' ? controller.readyExport() : null
  const exportArtifact = readyExport?.kind === 'blob' ? readyExport : null
  const savedExport = readyExport?.kind === 'saved' ? readyExport : null
  const downloadName = exportFilename(
    snapshot.metadata?.name ?? '',
    exportArtifact?.format ?? runFormat,
  )
  const download = useExportDownload(exportArtifact, downloadName, DOWNLOAD_URL_API)
  const footerAnnouncement = useFooterAnnouncement(snapshot.status, snapshot.export.status)
  const exportPhase = exportPhaseOf(
    snapshot.export.status,
    download !== null || savedExport !== null,
  )
  const rampBlocked = rampProblem(settings.mode, rampDraft) !== null
  const colourBlocked =
    colourProblem(foregroundDraft, 'foreground') !== null ||
    colourProblem(backgroundDraft, 'background') !== null
  const draftBlocker: DraftBlocker | null = rampBlocked ? 'ramp' : colourBlocked ? 'colour' : null
  const draftBlocked = draftBlocker !== null
  const canExport = hasSource && !isBusy && !draftBlocked
  const exportBlock = canExport
    ? null
    : exportBlockedBy(snapshot.status, hasSource, draftBlocker, exportFormat)
  const thresholdApplies = settings.mode === 'binary'

  function updateSetting(key: 'cellSize' | 'fontSize' | 'threshold', value: string) {
    controller.updateSettings({ ...settings, [key]: Number(value) })
  }

  function updateMode(mode: RenderModeName) {
    controller.updateSettings({ ...settings, mode })
  }

  /** Only a valid draft reaches the controller; an invalid one leaves the accepted ramp in use. */
  function updateRampDraft(draft: string) {
    setRampDraft(draft)
    if (validateRamp(draft) === null && draft !== settings.ramp) {
      controller.updateSettings({ ...settings, ramp: draft })
    }
  }

  function updateColourDraft(role: ColourRole, draft: string) {
    if (role === 'foreground') setForegroundDraft(draft)
    else setBackgroundDraft(draft)
  }

  /** Only a valid colour reaches the controller; an invalid draft leaves the accepted colour in use. */
  function updateColour(role: ColourRole, value: string) {
    if (colourProblem(value, role) === null && value !== settings[role]) {
      controller.updateSettings({ ...settings, [role]: value })
    }
  }

  function updateColorMode(colorMode: ColorModeName) {
    controller.updateSettings({ ...settings, colorMode })
  }

  function updateContrast(contrast: number) {
    controller.updateSettings({ ...settings, contrast })
  }

  function updateInvert(invert: boolean) {
    controller.updateSettings({ ...settings, invert })
  }

  function renderArtwork() {
    if (!draftBlocked) flow.renderArtwork()
  }

  function startExport() {
    if (!canExport) return
    setRunFormat(exportFormat)
    controller.startExport(exportFormat).catch(() => undefined)
  }

  return (
    <div
      className={`app-shell ${flow.dragActive ? 'is-dragging' : ''}`}
      onDragEnter={flow.onDragEnter}
      onDragOver={flow.onDragOver}
      onDragLeave={flow.onDragLeave}
      onDrop={flow.onDrop}
    >
      <a className="skip-link" href="#main-content">
        Skip to workspace
      </a>
      <input
        ref={flow.inputRef}
        className="visually-hidden"
        type="file"
        name="image"
        accept={IMPORT_ACCEPT}
        tabIndex={-1}
        aria-label="Choose an image file"
        aria-describedby={hintId}
        onChange={flow.onInputChange}
      />
      <header className="app-header">
        <a className="brand" href="/" aria-label="Image to Binary home">
          <span className="brand-mark" aria-hidden="true">
            ▦
          </span>
          <span>Image to Binary</span>
        </a>
        <div className="header-actions">
          <button className="button button-primary" type="button" onClick={flow.openPicker}>
            Open image
          </button>
          <button className="button button-secondary" type="button" onClick={flow.loadDemo}>
            Load demo
          </button>
          <button className="button button-quiet" type="button" onClick={flow.reset}>
            Reset
          </button>
          <button
            className="icon-button"
            type="button"
            aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} theme`}
            onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
          >
            <span aria-hidden="true">{theme === 'dark' ? '☼' : '☾'}</span>
          </button>
        </div>
      </header>

      <main className="workspace" id="main-content">
        <section className="canvas-column" aria-labelledby="canvas-heading">
          <div className="canvas-head">
            <div className="canvas-toolbar">
              <div>
                <p className="eyebrow">Artifact workspace</p>
                <h1 id="canvas-heading">{isOriginal ? 'Original image' : 'Artwork preview'}</h1>
              </div>
              <div className="view-toggle" role="group" aria-label="Preview view">
                <button
                  type="button"
                  className={isOriginal ? 'is-selected' : undefined}
                  aria-pressed={isOriginal}
                  disabled={!hasSource}
                  onClick={() => flow.selectView('original')}
                >
                  Original
                </button>
                <button
                  type="button"
                  className={isOriginal ? undefined : 'is-selected'}
                  aria-pressed={!isOriginal}
                  disabled={!hasSource || draftBlocked}
                  aria-describedby={draftBlocked ? artworkReasonId : undefined}
                  onClick={() => flow.selectView('artwork')}
                >
                  Artwork
                </button>
              </div>
            </div>
            {draftBlocker !== null && (
              <p className="field-note" id={artworkReasonId}>
                {DRAFT_BLOCK_ARTWORK[draftBlocker]}
              </p>
            )}
            {isBusy && snapshot.status !== 'exporting' && (
              <div className="busy-row">
                <progress aria-label={busyLabel} />
                <span>{busyLabel}</span>
                <button className="button button-secondary" type="button" onClick={flow.cancel}>
                  Cancel
                </button>
              </div>
            )}
            {errorMessage && (
              <div className="error-panel" role="alert">
                <p className="error-text">{errorMessage}</p>
                <p className="error-guidance">{IMPORT_GUIDANCE}</p>
                <button className="button button-secondary" type="button" onClick={flow.openPicker}>
                  Choose another image
                </button>
              </div>
            )}
          </div>
          <div className={`artifact-frame ${shown ? 'has-fixture' : ''}`}>
            {shown ? (
              <img
                className="fixture-preview"
                src={shown.url}
                alt={isOriginal ? 'Original image' : 'Binary artwork preview'}
                width={shown.width}
                height={shown.height}
              />
            ) : (
              <div className="empty-canvas">
                <span className="empty-glyph" aria-hidden="true">
                  ▦
                </span>
                <p>
                  {!hasSource
                    ? 'Open an image or drop one anywhere in this window.'
                    : isBusy
                      ? `Preparing the ${viewName}…`
                      : `The ${viewName} has not been rendered yet.`}
                </p>
                {!hasSource && (
                  <div className="empty-actions">
                    <button
                      className="button button-primary"
                      type="button"
                      onClick={flow.openPicker}
                    >
                      Open image
                    </button>
                    <button
                      className="button button-secondary"
                      type="button"
                      onClick={flow.loadDemo}
                    >
                      Load demo
                    </button>
                  </div>
                )}
                {hasSource && !isBusy && isOriginal && (
                  <button
                    className="button button-secondary"
                    type="button"
                    onClick={() => flow.selectView('original')}
                  >
                    Show original
                  </button>
                )}
                {hasSource && !isBusy && !isOriginal && (
                  <button
                    className="button button-secondary"
                    type="button"
                    onClick={renderArtwork}
                    disabled={draftBlocked}
                  >
                    Render artwork
                  </button>
                )}
              </div>
            )}
          </div>
          <p className="canvas-caption" id={hintId}>
            {flow.dragActive
              ? 'Release to open this image.'
              : 'Drop one AVIF, BMP, GIF, HEIC, HEIF, JPEG, PNG, TIFF or WebP image anywhere in this window, or use Open image.'}
          </p>
        </section>

        <aside
          className={`inspector ${inspectorOpen ? 'is-open' : 'is-closed'}`}
          aria-label="Rendering inspector"
        >
          <button
            className="inspector-toggle"
            type="button"
            aria-expanded={inspectorOpen}
            aria-controls="inspector-panel"
            onClick={() => setInspectorOpen((open) => !open)}
          >
            <span>Inspector</span>
            <span aria-hidden="true">{inspectorOpen ? '−' : '+'}</span>
          </button>
          <div className="inspector-content" id="inspector-panel">
            <section className="control-section" aria-labelledby="rendering-heading">
              <div className="section-heading">
                <h2 id="rendering-heading">Rendering</h2>
              </div>
              <label className="field-label" htmlFor="cell-size">
                Cell size <output>{settings.cellSize}px</output>
              </label>
              <input
                id="cell-size"
                name="cellSize"
                autoComplete="off"
                type="range"
                min={MIN_CELL_SIZE}
                max={MAX_CELL_SIZE}
                value={settings.cellSize}
                onChange={(event) => updateSetting('cellSize', event.target.value)}
              />
              <label className="field-label" htmlFor="font-size">
                Font size <output>{settings.fontSize}px</output>
              </label>
              <input
                id="font-size"
                name="fontSize"
                autoComplete="off"
                type="range"
                min={MIN_FONT_SIZE}
                max={MAX_FONT_SIZE}
                value={settings.fontSize}
                onChange={(event) => updateSetting('fontSize', event.target.value)}
              />
              <RenderModeFields
                mode={settings.mode}
                rampDraft={rampDraft}
                onModeChange={updateMode}
                onRampDraftChange={updateRampDraft}
              />
              <label className="field-label" htmlFor="threshold">
                Threshold <output>{settings.threshold}</output>
              </label>
              <input
                id="threshold"
                name="threshold"
                autoComplete="off"
                type="range"
                min={MIN_THRESHOLD}
                max={MAX_THRESHOLD}
                value={settings.threshold}
                disabled={!thresholdApplies}
                aria-describedby="threshold-note"
                onChange={(event) => updateSetting('threshold', event.target.value)}
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
                onColourChange={updateColour}
                onColourDraftChange={updateColourDraft}
                onColorModeChange={updateColorMode}
                onContrastChange={updateContrast}
                onInvertChange={updateInvert}
              />
              <button
                className="button button-primary button-wide"
                type="button"
                onClick={renderArtwork}
                disabled={!hasSource || isBusy || draftBlocked}
              >
                {snapshot.status === 'processing' ? 'Rendering…' : 'Render artwork'}
              </button>
            </section>
            <section className="control-section" aria-labelledby="metadata-heading">
              <div className="section-heading">
                <h2 id="metadata-heading">Metadata</h2>
              </div>
              {snapshot.metadata ? (
                <dl className="metadata-list">
                  <div>
                    <dt>Filename</dt>
                    <dd>{snapshot.metadata.name}</dd>
                  </div>
                  <div>
                    <dt>Dimensions</dt>
                    <dd>
                      {snapshot.metadata.width} × {snapshot.metadata.height} px
                    </dd>
                  </div>
                  {snapshot.metadata.inputBytes !== undefined && (
                    <div>
                      <dt>File size</dt>
                      <dd>{formatBytes(snapshot.metadata.inputBytes)}</dd>
                    </div>
                  )}
                  <div>
                    <dt>Source</dt>
                    <dd>{snapshot.metadata.source === 'demo' ? 'Built-in demo' : 'Local file'}</dd>
                  </div>
                  {snapshot.original && (
                    <div>
                      <dt>Original preview</dt>
                      <dd>
                        {snapshot.original.width} × {snapshot.original.height} px
                      </dd>
                    </div>
                  )}
                  {snapshot.preview && (
                    <div>
                      <dt>Artwork preview</dt>
                      <dd>
                        {snapshot.preview.width} × {snapshot.preview.height} px
                      </dd>
                    </div>
                  )}
                </dl>
              ) : (
                <p className="section-help">Open an image to inspect its metadata.</p>
              )}
            </section>
            <ExportPanel
              phase={exportPhase}
              canExport={canExport}
              disabledReason={exportBlock}
              progress={snapshot.export.progress}
              byteLength={snapshot.export.byteLength}
              error={snapshot.export.error}
              download={download}
              savedName={savedExport?.name ?? null}
              format={exportFormat}
              onFormatChange={setExportFormat}
              shownFormat={runFormat}
              onExport={startExport}
              onCancel={flow.cancel}
            />
          </div>
        </aside>
      </main>

      <footer className="status-bar">
        <span className={`status-dot ${hasSource ? 'is-active' : ''}`} aria-hidden="true" />
        <span aria-hidden="true">{footerStatusLabel(snapshot.status)}</span>
        <span className="status-note">Images are processed locally in your browser.</span>
        <span className="visually-hidden" role="status" aria-live="polite">
          {footerAnnouncement}
        </span>
      </footer>
    </div>
  )
}
