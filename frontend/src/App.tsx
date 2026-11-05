import { useEffect, useId, useState } from 'react'
import { IMPORT_ACCEPT, IMPORT_GUIDANCE, formatBytes, useImportFlow } from './useImportFlow'
import { useStudio } from './useStudio'

type Theme = 'dark' | 'light'

function getInitialTheme(): Theme {
  try {
    const storedTheme = window.localStorage.getItem('image-to-binary-theme')
    if (storedTheme === 'light' || storedTheme === 'dark') return storedTheme
  } catch {
    return 'dark'
  }
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

function statusLabel(status: string): string {
  switch (status) {
    case 'idle':
      return 'Ready for an image'
    case 'importing':
      return 'Importing image'
    case 'processing':
      return 'Rendering preview'
    case 'exporting':
      return 'Preparing export'
    case 'cancelled':
      return 'Operation cancelled'
    case 'error':
      return 'Action needs attention'
    case 'disposed':
      return 'Studio closed'
    default:
      return 'Preview ready'
  }
}

export default function App() {
  const { controller, snapshot } = useStudio()
  const flow = useImportFlow(controller)
  const hintId = useId()
  const [theme, setTheme] = useState<Theme>(getInitialTheme)
  const [inspectorOpen, setInspectorOpen] = useState(true)

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
  const isOriginal = flow.view === 'original'
  const shown = isOriginal ? snapshot.original : snapshot.preview
  const viewName = isOriginal ? 'original image' : 'artwork'
  const errorMessage = flow.localError ?? (snapshot.status === 'error' ? snapshot.error : null)
  const busyLabel =
    snapshot.status === 'importing'
      ? 'Importing image…'
      : snapshot.status === 'processing'
        ? 'Rendering preview…'
        : 'Preparing export…'

  function updateSetting(key: 'cellSize' | 'fontSize' | 'threshold', value: string) {
    controller.updateSettings({ ...settings, [key]: Number(value) })
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
                  disabled={!hasSource}
                  onClick={() => flow.selectView('artwork')}
                >
                  Artwork
                </button>
              </div>
            </div>
            {isBusy && (
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
                    onClick={flow.renderArtwork}
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
              : 'Drop one BMP, GIF, JPEG, PNG, TIFF or WebP image anywhere in this window, or use Open image.'}
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
                min="5"
                max="30"
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
                min="8"
                max="30"
                value={settings.fontSize}
                onChange={(event) => updateSetting('fontSize', event.target.value)}
              />
              <p className="field-note">Binary 0 / 1 output</p>
              <label className="field-label" htmlFor="threshold">
                Threshold <output>{settings.threshold}</output>
              </label>
              <input
                id="threshold"
                name="threshold"
                autoComplete="off"
                type="range"
                min="0"
                max="255"
                value={settings.threshold}
                onChange={(event) => updateSetting('threshold', event.target.value)}
              />
              <button
                className="button button-primary button-wide"
                type="button"
                onClick={flow.renderArtwork}
                disabled={!hasSource || isBusy}
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
            <section className="control-section export-section" aria-labelledby="export-heading">
              <div className="section-heading">
                <h2 id="export-heading">Export</h2>
              </div>
              <p className="section-help" id="export-help">
                Export is not available yet.
              </p>
              <button
                className="button button-secondary button-wide"
                type="button"
                aria-describedby="export-help"
                disabled
              >
                Export text
              </button>
            </section>
          </div>
        </aside>
      </main>

      <footer className="status-bar" role="status" aria-live="polite">
        <span className={`status-dot ${hasSource ? 'is-active' : ''}`} aria-hidden="true" />
        <span>{statusLabel(snapshot.status)}</span>
        <span className="status-note">Images are processed locally in your browser.</span>
      </footer>
    </div>
  )
}
