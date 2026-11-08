import { useEffect, useId, useRef, useState } from 'react'
import { ComparisonCanvas } from './ComparisonCanvas'
import type { ViewMode } from './comparisonGeometry'
import { ExportPanel } from './export/ExportPanel'
import { exportFilename } from './export/downloadName'
import { EXPORT_FORMAT_WORDS } from './export/exportFormats'
import type { ExportPhase } from './export/exportFocus'
import { footerStatusLabel, useFooterAnnouncement } from './export/footerAnnouncement'
import { useExportDownload } from './export/useExportDownload'
import { defaultUrlApi } from './processing/demoAdapter'
import type { NativeFiles } from './processing/nativeFiles'
import type { MenuAction, NativeMenu } from './processing/nativeMenu'
import type { ExportFormat, ExportStatus } from './processing/types'
import { validateRamp } from './processing/protocol'
import type { ColorModeName, ColourRole, RenderModeName } from './processing/protocol'
import type { StudioController } from './state/controller'
import { IMPORT_ACCEPT, IMPORT_GUIDANCE, useImportFlow } from './useImportFlow'
import { useStudio } from './useStudio'
import { useAutoRender } from './useAutoRender'
import {
  focusReachable,
  isBusyStatus,
  isFocusLost,
  menuStateOf,
  runStudioAction,
  shortcutAction,
  studioGates,
} from './studioActions'
import type { StudioAction, StudioGates, StudioHandlers } from './studioActions'
import { syncMenuState } from './processing/nativeMenu'
import { Button, SegmentedChoice, StatusRegion } from './ui'
import type { SegmentedOption } from './ui'
import {
  currentSystemTheme,
  LIGHT_SCHEME_QUERY,
  readThemeChoice,
  writeThemeChoice,
  THEME_COLOUR,
} from './appearance'
import type { Theme, ThemeChoice } from './appearance'
import { EmptyCanvas } from './EmptyCanvas'
import { MetadataSection } from './MetadataSection'
import { RenderingSection } from './RenderingSection'
import type { SizeSetting } from './RenderingSection'
import { StudioStatus } from './StudioStatus'
import { rampProblem } from './RenderModeFields'
import { colourProblem } from './RenderStyleFields'

const DOWNLOAD_URL_API = defaultUrlApi()
const THEME_OPTIONS: readonly SegmentedOption<ThemeChoice>[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

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
    const base = getComputedStyle(document.documentElement).getPropertyValue('--base').trim()
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute('content', base === '' ? THEME_COLOUR[theme] : base)
  }, [theme])

  const hasSource = snapshot.metadata !== null
  const isBusy = isBusyStatus(snapshot.status)
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
  const view = flow.view
  const isOriginal = view === 'original'
  const viewReady =
    view === 'original'
      ? snapshot.original !== null
      : view === 'artwork'
        ? snapshot.preview !== null
        : snapshot.original !== null && snapshot.preview !== null
  const viewName = isOriginal
    ? 'original image'
    : view === 'artwork'
      ? 'artwork'
      : 'split comparison'
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
  const autoRender = useAutoRender({
    controller,
    snapshot,
    draftBlocked,
    renderQuietly: flow.renderQuietly,
  })
  const artworkRefreshing = snapshot.artworkUpdating || autoRender.pending
  const gates = studioGates({
    hasSource,
    busy: isBusy,
    updating: snapshot.artworkUpdating,
    draftBlocked,
  })
  const canExport = gates.export
  const exportBlock = canExport
    ? null
    : exportBlockedBy(snapshot.status, hasSource, draftBlocker, exportFormat)
  const thresholdApplies = settings.mode === 'binary'

  function updateSetting(key: SizeSetting, value: number) {
    controller.updateSettings({ ...settings, [key]: value })
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
    return studioGates({
      hasSource: current.metadata !== null,
      busy: isBusyStatus(current.status),
      updating: current.artworkUpdating,
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
    artwork: () => {
      autoRender.clear()
      flow.selectView('artwork')
    },
    split: () => {
      autoRender.clear()
      flow.selectView('split')
    },
    render: () => {
      autoRender.clear()
      flow.renderArtwork()
    },
    cancel: () => {
      autoRender.suppress()
      flow.cancel()
    },
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
    menuState.split,
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
      if (action === 'cancel') autoRender.suppress()
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

  const draftBlockedText = draftBlocker === null ? undefined : DRAFT_BLOCK_ARTWORK[draftBlocker]
  const viewOptions: readonly SegmentedOption<ViewMode>[] = [
    { value: 'original', label: 'Original', disabled: !gates.original },
    {
      value: 'artwork',
      label: 'Artwork',
      disabled: !gates.artwork,
      description: draftBlockedText,
    },
    {
      value: 'split',
      label: 'Split',
      disabled: !gates.split,
      description: draftBlockedText,
    },
  ]
  const statusBusyLabel = isBusy && snapshot.status !== 'exporting' ? busyLabel : null
  const artworkShown = viewReady && !isOriginal
  const refreshing = artworkShown && artworkRefreshing

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
          <Button ref={openRef} variant="primary" onClick={openImage}>
            Open image
          </Button>
          <Button variant="secondary" onClick={flow.loadDemo}>
            Load demo
          </Button>
          <Button variant="quiet" onClick={resetStudio}>
            Reset
          </Button>
          <SegmentedChoice
            legend="Appearance"
            legendHidden
            className="theme-choice"
            options={THEME_OPTIONS}
            value={themeChoice}
            onChange={chooseTheme}
          />
        </div>
      </header>

      <main className="workspace" id="main-content">
        <section className="canvas-column" aria-labelledby="canvas-heading">
          <div className="canvas-head">
            <div className="canvas-toolbar">
              <div>
                <p className="eyebrow">Artifact workspace</p>
                <h1 id="canvas-heading" ref={headingRef} tabIndex={-1} data-focus-fallback="true">
                  {isOriginal
                    ? 'Original image'
                    : view === 'artwork'
                      ? 'Artwork preview'
                      : 'Split comparison'}
                </h1>
              </div>
              <SegmentedChoice
                legend="Preview view"
                legendHidden
                className="view-toggle"
                columns={3}
                options={viewOptions}
                value={view}
                onChange={(next) => runAction(next)}
              />
            </div>
            {draftBlocker !== null && (
              <p className="field-note" id={artworkReasonId}>
                {DRAFT_BLOCK_ARTWORK[draftBlocker]}
              </p>
            )}
            <StudioStatus
              busyLabel={statusBusyLabel}
              onCancel={() => runAction('cancel')}
              errorMessage={errorMessage}
              guidance={IMPORT_GUIDANCE}
              onChooseAnother={openImage}
            />
          </div>
          <div
            className={`artifact-frame ${viewReady ? 'has-fixture' : ''} ${refreshing ? 'is-refreshing' : ''}`}
            aria-busy={refreshing || undefined}
          >
            {snapshot.metadata !== null && (
              <div className="canvas-slot" hidden={!viewReady}>
                <ComparisonCanvas
                  view={view}
                  source={{ width: snapshot.metadata.width, height: snapshot.metadata.height }}
                  sourceToken={snapshot.metadata}
                  original={snapshot.original}
                  artwork={snapshot.preview}
                  originalAlt="Original image"
                  artworkAlt="Binary artwork preview"
                  visible={viewReady}
                />
              </div>
            )}
            {!viewReady && (
              <EmptyCanvas
                hasSource={hasSource}
                isBusy={isBusy}
                isOriginal={isOriginal}
                viewName={viewName}
                renderEnabled={gates.render}
                onOpen={openImage}
                onLoadDemo={flow.loadDemo}
                onShowOriginal={() => flow.selectView('original')}
                rendering={artworkRefreshing}
                onRender={() => runAction('render')}
              />
            )}
            <div className="canvas-status" role="status" aria-live="polite">
              {refreshing && <span className="updating-badge">Updating…</span>}
            </div>
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
            <RenderingSection
              settings={settings}
              rampDraft={rampDraft}
              foregroundDraft={foregroundDraft}
              backgroundDraft={backgroundDraft}
              thresholdApplies={thresholdApplies}
              renderEnabled={gates.render}
              rendering={snapshot.status === 'processing'}
              onSizeChange={updateSetting}
              onModeChange={updateMode}
              onRampDraftChange={updateRampDraft}
              onColourChange={updateColour}
              onColourDraftChange={updateColourDraft}
              onColorModeChange={updateColorMode}
              onContrastChange={updateContrast}
              onInvertChange={updateInvert}
              onRender={() => runAction('render')}
            />
            <MetadataSection
              metadata={snapshot.metadata}
              original={snapshot.original}
              preview={snapshot.preview}
            />
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
        <span className="status-note">Images are processed locally on this device.</span>
        <StatusRegion className="visually-hidden">{footerAnnouncement}</StatusRegion>
      </footer>
    </div>
  )
}
