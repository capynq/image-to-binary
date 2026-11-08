import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, KeyboardEvent, PointerEvent, RefObject } from 'react'
import { PreviewImage } from './PreviewImage'
import {
  comparisonFrame,
  cssScale,
  splitKeyValue,
  splitLayout,
  splitValueText,
  viewportKeyCommand,
  wheelCommand,
} from './comparisonGeometry'
import type { Size, ViewMode, ViewportCommand } from './comparisonGeometry'
import {
  INITIAL_VIEWPORT,
  applyViewportCommand,
  effectiveZoom,
  needsRefit,
  panBy,
  setSplit,
  stageOffset,
  zoomReadout,
} from './comparisonViewport'
import type { Point, ViewportLayout, ViewportState } from './comparisonViewport'
import type { PreviewSnapshot } from './processing/types'
import { Button } from './ui'

export interface ComparisonCanvasProps {
  readonly view: ViewMode
  /** Source dimensions from the metadata; both images map onto this source space. */
  readonly source: Size
  /** Identity of the imported source; a different object refits the view. */
  readonly sourceToken: object
  readonly original: PreviewSnapshot | null
  readonly artwork: PreviewSnapshot | null
  readonly originalAlt: string
  readonly artworkAlt: string
  /** False while the host hides the canvas; the viewport is measured again once it is shown. */
  readonly visible?: boolean
}

interface Pointer {
  readonly pointerId: number
  x: number
  y: number
}

const IMAGE_BASE: CSSProperties = {
  position: 'absolute',
  left: 0,
  top: 0,
  maxWidth: 'none',
  maxHeight: 'none',
  minWidth: 0,
  minHeight: 0,
}

/**
 * Measures an element's client box before the first paint, so the stage is never shown at its
 * unmeasured size, and follows later size changes through a ResizeObserver. A hidden element
 * measures zero, so it is measured again whenever `visible` turns true, before that paint.
 */
function useElementSize(ref: RefObject<HTMLElement | null>, visible: boolean): Size {
  const [size, setSize] = useState<Size>({ width: 0, height: 0 })
  useLayoutEffect(() => {
    const element = ref.current
    if (element === null) return undefined
    const measure = () => {
      const next = { width: element.clientWidth, height: element.clientHeight }
      setSize((previous) =>
        previous.width === next.width && previous.height === next.height ? previous : next,
      )
    }
    measure()
    if (typeof ResizeObserver !== 'function') {
      window.addEventListener('resize', measure)
      return () => window.removeEventListener('resize', measure)
    }
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref, visible])
  return size
}

/**
 * The comparison canvas: the original, the artwork, or a split of both over one source region,
 * with fit, 100%, zoom, pan and a split divider. Viewport state is UI-only and refits on a new
 * source or a change in the frame's dimensions; the canvas never calls the controller.
 *
 * Images are drawn with `object-fit: fill` in the split and artwork views (see styles.css), so the
 * box is the image: the split stretches the artwork to the source extent, and the artwork view
 * shows its natural pixels with no letterbox. The original view keeps the default fit.
 */
export function ComparisonCanvas({
  view,
  source,
  sourceToken,
  original,
  artwork,
  originalAlt,
  artworkAlt,
  visible = true,
}: ComparisonCanvasProps) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<HTMLDivElement>(null)
  const readoutId = useId()
  const viewportSize = useElementSize(viewportRef, visible)
  const frame = comparisonFrame({
    view,
    source,
    original,
    artwork,
  })
  const dimensions = `${frame.originalExtent.width}x${frame.originalExtent.height}:${frame.artworkExtent.width}x${frame.artworkExtent.height}`
  const [seen, setSeen] = useState({ source: sourceToken, dimensions })
  const [viewport, setViewport] = useState<ViewportState>(INITIAL_VIEWPORT)
  const [dragging, setDragging] = useState(false)
  const drag = useRef<Pointer | null>(null)
  const divider = useRef<number | null>(null)

  if (needsRefit(seen, { source: sourceToken, dimensions })) {
    setSeen({ source: sourceToken, dimensions })
    setViewport(INITIAL_VIEWPORT)
  }

  const layout: ViewportLayout = {
    viewport: viewportSize,
    frame: frame.frame,
    naturalPerSource: frame.naturalPerSource,
  }
  const latest = useRef({ layout, viewport })
  useEffect(() => {
    latest.current = { layout, viewport }
  })

  const scale = cssScale(effectiveZoom(viewport, layout), frame.naturalPerSource)
  const offset = stageOffset(viewport, layout)
  const stage = { width: frame.frame.width * scale, height: frame.frame.height * scale }
  const overflows = stage.width > viewportSize.width || stage.height > viewportSize.height
  const boxes = splitLayout(
    frame.frame,
    frame.originalExtent,
    frame.artworkExtent,
    scale,
    viewport.split,
  )
  const altText =
    view === 'original'
      ? originalAlt
      : view === 'artwork'
        ? artworkAlt
        : `${originalAlt}, left of the split, and ${artworkAlt}, right of it`

  function apply(command: ViewportCommand, anchor?: Point) {
    setViewport((previous) =>
      applyViewportCommand(previous, latest.current.layout, command, anchor),
    )
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget) return
    if (event.altKey || event.ctrlKey || event.metaKey) return
    const command = viewportKeyCommand(event.key)
    if (command === null) return
    if (command.kind === 'pan' && !panMoves(command.dx, command.dy)) return
    event.preventDefault()
    apply(command)
  }

  useEffect(() => {
    const element = viewportRef.current
    if (element === null) return undefined
    const onWheel = (event: globalThis.WheelEvent) => {
      const current = latest.current
      const command = wheelCommand(event, current.layout.viewport)
      if (command.kind === 'zoom') {
        event.preventDefault()
        const rect = element.getBoundingClientRect()
        const anchor = {
          x: event.clientX - rect.left - element.clientLeft,
          y: event.clientY - rect.top - element.clientTop,
        }
        setViewport((previous) => applyViewportCommand(previous, current.layout, command, anchor))
        return
      }
      if (!panMovesIn(current, command.dx, command.dy)) return
      event.preventDefault()
      setViewport((previous) => applyViewportCommand(previous, current.layout, command))
    }
    element.addEventListener('wheel', onWheel, { passive: false })
    return () => element.removeEventListener('wheel', onWheel)
  }, [])

  useEffect(
    () => () => {
      drag.current = null
      divider.current = null
    },
    [],
  )

  function panMovesIn(
    current: { layout: ViewportLayout; viewport: ViewportState },
    dx: number,
    dy: number,
  ): boolean {
    return panBy(current.viewport, current.layout, dx, dy) !== current.viewport
  }

  function panMoves(dx: number, dy: number): boolean {
    return panMovesIn(latest.current, dx, dy)
  }

  function onViewportPointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0 || !overflows) return
    drag.current = { pointerId: event.pointerId, x: event.clientX, y: event.clientY }
    event.currentTarget.setPointerCapture(event.pointerId)
    setDragging(true)
  }

  function onViewportPointerMove(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current
    if (current === null || current.pointerId !== event.pointerId) return
    if (event.buttons === 0) {
      endDrag(event)
      return
    }
    const dx = event.clientX - current.x
    const dy = event.clientY - current.y
    current.x = event.clientX
    current.y = event.clientY
    apply({ kind: 'pan', dx, dy })
  }

  function endDrag(event: PointerEvent<HTMLDivElement>) {
    if (drag.current?.pointerId !== event.pointerId) return
    drag.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    setDragging(false)
  }

  function onDividerPointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return
    event.stopPropagation()
    divider.current = event.pointerId
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  function onDividerPointerMove(event: PointerEvent<HTMLDivElement>) {
    if (divider.current !== event.pointerId) return
    if (event.buttons === 0) {
      endDividerDrag(event)
      return
    }
    const rect = stageRef.current?.getBoundingClientRect()
    if (rect === undefined || rect.width <= 0) return
    const share = ((event.clientX - rect.left) / rect.width) * 100
    setViewport((previous) => setSplit(previous, share))
  }

  function endDividerDrag(event: PointerEvent<HTMLDivElement>) {
    if (divider.current !== event.pointerId) return
    divider.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }

  function onDividerKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const next = splitKeyValue(viewport.split, event.key, event.shiftKey)
    if (next === null) return
    event.preventDefault()
    event.stopPropagation()
    setViewport((previous) => setSplit(previous, next))
  }

  return (
    <section className="comparison-canvas" aria-label="Preview canvas">
      <div className="viewport-controls" role="group" aria-label="Zoom controls">
        <Button variant="secondary" onClick={() => apply({ kind: 'zoom', factor: 1 / 1.25 })}>
          Zoom out
        </Button>
        <Button variant="secondary" onClick={() => apply({ kind: 'zoom', factor: 1.25 })}>
          Zoom in
        </Button>
        <Button variant="secondary" onClick={() => apply({ kind: 'fit' })}>
          Fit
        </Button>
        <Button variant="secondary" onClick={() => apply({ kind: 'actual' })}>
          100%
        </Button>
        <output id={readoutId} className="zoom-readout">
          {zoomReadout(viewport, layout)}
        </output>
      </div>
      <div
        ref={viewportRef}
        className="comparison-viewport"
        role="group"
        aria-label={`Zoomable view: ${altText}`}
        aria-describedby={readoutId}
        tabIndex={0}
        data-pannable={overflows ? 'true' : undefined}
        data-dragging={dragging ? 'true' : undefined}
        onKeyDown={onKeyDown}
        onPointerDown={onViewportPointerDown}
        onPointerMove={onViewportPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onLostPointerCapture={endDrag}
      >
        <div
          ref={stageRef}
          className="comparison-stage"
          data-view={view}
          style={{
            width: stage.width,
            height: stage.height,
            transform: `translate(${offset.x}px, ${offset.y}px)`,
          }}
        >
          {view === 'original' && original !== null && (
            <PreviewImage
              src={original.url}
              alt={originalAlt}
              width={original.width}
              height={original.height}
              style={{ ...IMAGE_BASE, width: boxes.original.width, height: boxes.original.height }}
            />
          )}
          {view === 'artwork' && artwork !== null && (
            <PreviewImage
              src={artwork.url}
              alt={artworkAlt}
              width={artwork.width}
              height={artwork.height}
              style={{ ...IMAGE_BASE, width: stage.width, height: stage.height }}
            />
          )}
          {view === 'split' && original !== null && artwork !== null && (
            <>
              <div
                className="comparison-clip"
                style={{ left: 0, width: boxes.seam, height: stage.height }}
              >
                <PreviewImage
                  src={original.url}
                  alt={originalAlt}
                  width={original.width}
                  height={original.height}
                  style={{
                    ...IMAGE_BASE,
                    width: boxes.original.width,
                    height: boxes.original.height,
                  }}
                />
              </div>
              <div
                className="comparison-clip comparison-artwork"
                style={{
                  left: boxes.seam,
                  width: Math.max(0, stage.width - boxes.seam),
                  height: stage.height,
                }}
              >
                <PreviewImage
                  src={artwork.url}
                  alt={artworkAlt}
                  width={artwork.width}
                  height={artwork.height}
                  style={{
                    ...IMAGE_BASE,
                    left: -boxes.seam,
                    width: boxes.artwork.width,
                    height: boxes.artwork.height,
                  }}
                />
              </div>
              <div
                role="slider"
                tabIndex={0}
                className="comparison-divider"
                aria-label="Split position"
                aria-orientation="horizontal"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(viewport.split)}
                aria-valuetext={splitValueText(viewport.split)}
                style={{ left: boxes.seam, touchAction: 'none' }}
                onKeyDown={onDividerKeyDown}
                onPointerDown={onDividerPointerDown}
                onPointerMove={onDividerPointerMove}
                onPointerUp={endDividerDrag}
                onPointerCancel={endDividerDrag}
                onLostPointerCapture={endDividerDrag}
              />
            </>
          )}
        </div>
      </div>
    </section>
  )
}
