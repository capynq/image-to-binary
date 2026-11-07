import { useEffect, useId, useRef, useState } from 'react'
import { RenderModeFields, rampProblem } from './RenderModeFields'
import { RenderStyleFields, colourProblem } from './RenderStyleFields'
import { ExportPanel } from './export/ExportPanel'
import { PreviewImage } from './PreviewImage'
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
import type { MenuAction, NativeMenu } from './processing/nativeMenu'
import type { ExportFormat, ExportStatus } from './processing/types'
import { validateRamp } from './processing/protocol'
import type { ColorModeName, ColourRole, RenderModeName } from './processing/protocol'
import type { StudioController } from './state/controller'
import { IMPORT_ACCEPT, IMPORT_GUIDANCE, formatBytes, useImportFlow } from './useImportFlow'
import { useStudio } from './useStudio'
import {
  focusReachable,
  isFocusLost,
  menuStateOf,
  runStudioAction,
  shortcutAction,
  studioGates,
} from './studioActions'
import type { StudioAction, StudioGates, StudioHandlers } from './studioActions'
import { syncMenuState } from './processing/nativeMenu'

type Theme = 'dark' | 'light'
/** The appearance the user chose; `system` follows the operating system and is stored explicitly. */
type ThemeChoice = 'system' | Theme

const DOWNLOAD_URL_API = defaultUrlApi()
const THEME_CHOICE_KEY = 'image-to-binary-theme-choice'
const LIGHT_SCHEME_QUERY = '(prefers-color-scheme: light)'
const THEME_OPTIONS: readonly { readonly value: ThemeChoice; readonly label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

/**
 * The stored appearance choice. Anything missing, unreadable or unknown means `system`. The
 * pre-5.3 key is never read: it was written on every load, so it cannot tell a real choice apart.
 */
function readThemeChoice(): ThemeChoice {
  try {
    const stored = window.localStorage.getItem(THEME_CHOICE_KEY)
    if (stored === 'light' || stored === 'dark' || stored === 'system') return stored
  } catch {
    return 'system'
  }
  return 'system'
}

function writeThemeChoice(choice: ThemeChoice): void {
  try {
    window.localStorage.setItem(THEME_CHOICE_KEY, choice)
  } catch {
    return
  }
}

function currentSystemTheme(): Theme {
  if (typeof window.matchMedia !== 'function') return 'dark'
  return window.matchMedia(LIGHT_SCHEME_QUERY).matches ? 'light' : 'dark'
}

function activeHtmlElement(): HTMLElement | null {
  const active = document.activeElement
  if (!(active instanceof HTMLElement) || active === document.body) return null
  return active
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
  /** Present only inside the desktop shell; the menu bar's actions and enabled state. */
  readonly menu?: NativeMenu | null
}

export default function App({ controller, native, menu = null }: AppProps) {
  const { snapshot } = useStudio(controller)
  const flow = useImportFlow(controller, native)
  const hintId = useId()
  const [themeChoice, setThemeChoice] = useState<ThemeChoice>(readThemeChoice)
  const [systemTheme, setSystemTheme] = useState<Theme>(currentSystemTheme)
  const theme: Theme = themeChoice === 'system' ? systemTheme : themeChoice
  const themeGroupName = useId()
  const [exportSettled, setExportSettled] = useState(0)
  const openRef = useRef<HTMLButtonElement>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)
  const [inspectorOpen, setInspectorOpen] = useState(true)
  const [rampDraft, setRampDraft] = useState(() => snapshot.settings.ramp)
  const [foregroundDraft, setForegroundDraft] = useState(() => snapshot.settings.foreground)
  const [backgroundDraft, setBackgroundDraft] = useState(() => snapshot.settings.background)
  const [exportFormat, setExportFormat] = useState<ExportFormat>('png')
  const [runFormat, setRunFormat] = useState<ExportFormat>('png')
  const artworkReasonId = useId()

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return undefined
    const query = window.matchMedia(LIGHT_SCHEME_QUERY)
    const follow = () => setSystemTheme(query.matches ? 'light' : 'dark')
    query.addEventListener('change', follow)
    return () => query.removeEventListener('change', follow)
  }, [])

  useEffect(() => {
    document.documentElement.dataset.theme = theme
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute('content', theme === 'dark' ? '#0e1118' : '#f3f5fa')
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
  const gates = studioGates({ hasSource, busy: isBusy, draftBlocked })
  const canExport = gates.export
  const exportBlock = canExport
    ? null
    : exportBlockedBy(snapshot.status, hasSource, draftBlocker, exportFormat)
  const thresholdApplies = settings.mode === 'binary'

  function updateSetting(key: 'cellSize' | 'fontSize' | 'threshold', value: string) {
    controller.updateSettings({ ...settings, [key]: Number(value) })
  }

  function chooseTheme(choice: ThemeChoice) {
    setThemeChoice(choice)
    writeThemeChoice(choice)
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

  function currentGates(): StudioGates {
    const current = controller.getSnapshot()
    const busyNow =
      current.status === 'importing' ||
      current.status === 'processing' ||
      current.status === 'exporting'
    return studioGates({
      hasSource: current.metadata !== null,
      busy: busyNow,
      draftBlocked: latest.current.draftBlocked,
    })
  }

  function startExport() {
    if (!currentGates().export) return
    setRunFormat(exportFormat)
    controller
      .startExport(exportFormat)
      .catch(() => undefined)
      .finally(() => setExportSettled((count) => count + 1))
  }

  /** Opens the picker and returns focus to the control that opened it once the dialog settles. */
  function openImage() {
    const origin = activeHtmlElement()
    void flow.openPicker().finally(() => {
      focusReachable(origin, openRef.current)
    })
  }

  function resetStudio() {
    flow.reset()
    focusReachable(openRef.current, headingRef.current)
  }

  const handlers: StudioHandlers = {
    open: openImage,
    export: startExport,
    original: () => flow.selectView('original'),
    artwork: () => flow.selectView('artwork'),
    render: () => flow.renderArtwork(),
    cancel: () => flow.cancel(),
  }
  const latest = useRef({ draftBlocked, handlers })
  useEffect(() => {
    latest.current = { draftBlocked, handlers }
  })

  /**
   * Gates are read from the controller at the moment of the action, not from the last render, so a
   * shortcut pressed before React commits the new busy state cannot start a second export.
   */
  function runAction(action: StudioAction): boolean {
    return runStudioAction(action, currentGates(), latest.current.handlers)
  }

  const menuState = menuStateOf(gates)
  useEffect(() => {
    if (menu === null) return undefined
    return syncMenuState(menu, menuState)
  }, [
    menu,
    menuState.export,
    menuState.render,
    menuState.original,
    menuState.artwork,
    menuState.cancel,
  ])

  useEffect(() => {
    if (menu === null) return undefined
    let active = true
    const stops: Array<() => void> = []
    const onAction = (action: MenuAction) => {
      if (!active) return
      const ran = runAction(action)
      const focusOnBody =
        document.activeElement === null || document.activeElement === document.body
      if ((!ran || action !== 'export') && focusOnBody) headingRef.current?.focus()
    }
    menu
      .listen(onAction)
      .then((stop) => {
        if (active) stops.push(stop)
        else stop()
      })
      .catch(() => undefined)
    return () => {
      active = false
      for (const stop of stops) stop()
    }
  }, [menu, controller])

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.defaultPrevented) return
      const action = shortcutAction(event)
      if (action === null) return
      if (runAction(action)) event.preventDefault()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [controller])

  const wasBusy = useRef(isBusy)
  const busyOrigin = useRef<HTMLElement | null>(null)
  const busyWasExport = useRef(false)
  useEffect(() => {
    const previous = wasBusy.current
    wasBusy.current = isBusy
    if (!previous && isBusy) {
      busyOrigin.current = activeHtmlElement()
      busyWasExport.current = snapshot.status === 'exporting'
    }
    if (!previous || isBusy) return
    const origin = busyOrigin.current
    busyOrigin.current = null
    if (busyWasExport.current || !isFocusLost(document.activeElement)) return
    focusReachable(origin, headingRef.current)
  }, [isBusy, snapshot.status])

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
          <button ref={openRef} className="button button-primary" type="button" onClick={openImage}>
            Open image
          </button>
          <button className="button button-secondary" type="button" onClick={flow.loadDemo}>
            Load demo
          </button>
          <button className="button button-quiet" type="button" onClick={resetStudio}>
            Reset
          </button>
          <fieldset className="theme-choice">
            <legend className="visually-hidden">Appearance</legend>
            {THEME_OPTIONS.map((option) => (
              <label
                key={option.value}
                className={`theme-option ${themeChoice === option.value ? 'is-selected' : ''}`}
              >
                <input
                  type="radio"
                  name={themeGroupName}
                  value={option.value}
                  checked={themeChoice === option.value}
                  onChange={() => chooseTheme(option.value)}
                />
                <span>{option.label}</span>
              </label>
            ))}
          </fieldset>
        </div>
      </header>

      <main className="workspace" id="main-content">
        <section className="canvas-column" aria-labelledby="canvas-heading">
          <div className="canvas-head">
            <div className="canvas-toolbar">
              <div>
                <p className="eyebrow">Artifact workspace</p>
                <h1 id="canvas-heading" ref={headingRef} tabIndex={-1} data-focus-fallback="true">
                  {isOriginal ? 'Original image' : 'Artwork preview'}
                </h1>
              </div>
              <div className="view-toggle" role="group" aria-label="Preview view">
                <button
                  type="button"
                  className={isOriginal ? 'is-selected' : undefined}
                  aria-pressed={isOriginal}
                  disabled={!gates.original}
                  onClick={() => runAction('original')}
                >
                  Original
                </button>
                <button
                  type="button"
                  className={isOriginal ? undefined : 'is-selected'}
                  aria-pressed={!isOriginal}
                  disabled={!gates.artwork}
                  aria-describedby={draftBlocked ? artworkReasonId : undefined}
                  onClick={() => runAction('artwork')}
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
                <button
                  className="button button-secondary"
                  type="button"
                  onClick={() => runAction('cancel')}
                >
                  Cancel
                </button>
              </div>
            )}
            {errorMessage && (
              <div className="error-panel" role="alert">
                <p className="error-text">{errorMessage}</p>
                <p className="error-guidance">{IMPORT_GUIDANCE}</p>
                <button className="button button-secondary" type="button" onClick={openImage}>
                  Choose another image
                </button>
              </div>
            )}
          </div>
          <div className={`artifact-frame ${shown ? 'has-fixture' : ''}`}>
            {shown ? (
              <PreviewImage
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
                    <button className="button button-primary" type="button" onClick={openImage}>
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
                    onClick={() => runAction('render')}
                    disabled={!gates.render}
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
              <div className="range-heading">
                <label className="field-label" htmlFor="cell-size">
                  Cell size
                </label>
                <output htmlFor="cell-size">{settings.cellSize}px</output>
              </div>
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
              <div className="range-heading">
                <label className="field-label" htmlFor="font-size">
                  Font size
                </label>
                <output htmlFor="font-size">{settings.fontSize}px</output>
              </div>
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
              <div className="range-heading">
                <label className="field-label" htmlFor="threshold">
                  Threshold
                </label>
                <output htmlFor="threshold">{settings.threshold}</output>
              </div>
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
                onClick={() => runAction('render')}
                disabled={!gates.render}
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
              onCancel={() => runAction('cancel')}
              focusRequest={exportSettled}
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
