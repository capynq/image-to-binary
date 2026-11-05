import { useEffect, useState } from 'react'
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
      return 'Loading demo fixture'
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

function runAction(action: () => Promise<void>): void {
  void action().catch(() => undefined)
}

export default function App() {
  const { controller, snapshot } = useStudio()
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
  const hasPreview = snapshot.preview !== null
  const isBusy =
    snapshot.status === 'importing' ||
    snapshot.status === 'processing' ||
    snapshot.status === 'exporting'
  const settings = snapshot.settings

  function updateSetting(key: 'cellSize' | 'fontSize' | 'threshold', value: string) {
    controller.updateSettings({ ...settings, [key]: Number(value) })
  }

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to workspace
      </a>
      <header className="app-header">
        <a className="brand" href="/" aria-label="Image to Binary home">
          <span className="brand-mark" aria-hidden="true">
            ▦
          </span>
          <span>Image to Binary</span>
        </a>
        <div className="header-actions">
          <button
            className="button button-primary"
            type="button"
            onClick={() => runAction(() => controller.loadDemo())}
          >
            Load demo
          </button>
          <button className="button button-quiet" type="button" onClick={() => controller.reset()}>
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
          <div className="canvas-toolbar">
            <div>
              <p className="eyebrow">Artifact workspace</p>
              <h1 id="canvas-heading">Artwork preview</h1>
            </div>
          </div>
          <div className={`artifact-frame ${hasPreview ? 'has-fixture' : ''}`}>
            {hasPreview && snapshot.preview ? (
              <img
                className="fixture-preview"
                src={snapshot.preview.url}
                alt="Binary artifact preview"
                width={snapshot.preview.width}
                height={snapshot.preview.height}
              />
            ) : (
              <div className="empty-canvas">
                <span className="empty-glyph" aria-hidden="true">
                  ▦
                </span>
                <p>
                  {hasSource
                    ? 'Render the loaded image to see its artifact.'
                    : 'Load a demo to place an image on the grid.'}
                </p>
                {!hasSource && (
                  <button
                    className="button button-secondary"
                    type="button"
                    onClick={() => runAction(() => controller.loadDemo())}
                  >
                    Load demo
                  </button>
                )}
                {hasSource && (
                  <button
                    className="button button-secondary"
                    type="button"
                    onClick={() => runAction(() => controller.preview())}
                    disabled={isBusy}
                  >
                    Render preview
                  </button>
                )}
              </div>
            )}
          </div>
          <p className="canvas-caption">Rendered by the Rust engine in a background worker.</p>
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
                onClick={() => runAction(() => controller.preview())}
                disabled={!hasSource || isBusy}
              >
                {isBusy && snapshot.status === 'processing' ? 'Rendering…' : 'Render preview'}
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
                    <dt>Source size</dt>
                    <dd>
                      {snapshot.metadata.width} × {snapshot.metadata.height} px
                    </dd>
                  </div>
                  <div>
                    <dt>Source</dt>
                    <dd>{snapshot.metadata.source}</dd>
                  </div>
                  {snapshot.preview && (
                    <div>
                      <dt>Preview size</dt>
                      <dd>
                        {snapshot.preview.width} × {snapshot.preview.height} px
                      </dd>
                    </div>
                  )}
                </dl>
              ) : (
                <p className="section-help">Load an image to inspect its metadata.</p>
              )}
            </section>
            <section className="control-section export-section" aria-labelledby="export-heading">
              <div className="section-heading">
                <h2 id="export-heading">Export</h2>
              </div>
              <p className="section-help" id="export-help">
                Export is not available yet. Text and PNG downloads arrive in later phases.
              </p>
              <button
                className="button button-secondary button-wide"
                type="button"
                aria-describedby="export-help"
                disabled
              >
                Export text
              </button>
              {isBusy && (
                <button
                  className="button button-quiet button-wide"
                  type="button"
                  onClick={() => controller.cancel()}
                >
                  Cancel
                </button>
              )}
            </section>
          </div>
        </aside>
      </main>

      <footer className="status-bar" role="status" aria-live="polite">
        <span className={`status-dot ${hasSource ? 'is-active' : ''}`} aria-hidden="true" />
        <span>{snapshot.error ?? statusLabel(snapshot.status)}</span>
        <span className="status-note">
          Rust worker renders a built-in demo image. File import and export come later.
        </span>
      </footer>
    </div>
  )
}
