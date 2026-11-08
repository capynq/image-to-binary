import { Button } from './ui'

export interface EmptyCanvasProps {
  readonly hasSource: boolean
  readonly isBusy: boolean
  readonly isOriginal: boolean
  readonly viewName: string
  readonly renderEnabled: boolean
  /** Artwork is being rendered or is queued to render after an edit. */
  readonly rendering: boolean
  readonly onOpen: () => void
  readonly onLoadDemo: () => void
  readonly onShowOriginal: () => void
  readonly onRender: () => void
}

/** The canvas placeholder shown before an image is open or before the artwork is rendered. */
export function EmptyCanvas({
  hasSource,
  isBusy,
  isOriginal,
  viewName,
  renderEnabled,
  rendering,
  onOpen,
  onLoadDemo,
  onShowOriginal,
  onRender,
}: EmptyCanvasProps) {
  return (
    <div className="empty-canvas">
      <span className="empty-glyph" aria-hidden="true">
        ▦
      </span>
      <p>
        {!hasSource
          ? 'Open an image or drop one anywhere in this window.'
          : isBusy
            ? `Preparing the ${viewName}…`
            : rendering && !isOriginal
              ? 'Rendering artwork…'
              : `The ${viewName} has not been rendered yet. Changes render automatically.`}
      </p>
      {!hasSource && (
        <div className="empty-actions">
          <Button variant="primary" onClick={onOpen}>
            Open image
          </Button>
          <Button variant="secondary" onClick={onLoadDemo}>
            Load demo
          </Button>
        </div>
      )}
      {hasSource && !isBusy && isOriginal && (
        <Button variant="secondary" onClick={onShowOriginal}>
          Show original
        </Button>
      )}
      {hasSource && !isBusy && !isOriginal && (
        <Button variant="secondary" disabled={!renderEnabled} onClick={onRender}>
          Render artwork
        </Button>
      )}
    </div>
  )
}
