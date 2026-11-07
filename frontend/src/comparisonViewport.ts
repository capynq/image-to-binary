import {
  SPLIT_DEFAULT,
  SPLIT_MAX,
  SPLIT_MIN,
  ZOOM_MAX,
  ZOOM_MIN,
  clamp,
  cssScale,
  fitZoom,
  placeAxis,
} from './comparisonGeometry'
import type { Size, ViewportCommand } from './comparisonGeometry'

/**
 * UI-only viewport state. `fit` follows the viewport size; otherwise `zoom` is relative to the
 * natural scale and `panX`/`panY` place the stage in viewport pixels. It never reaches the
 * controller, the settings, or the export request.
 */
export interface ViewportState {
  readonly fit: boolean
  readonly zoom: number
  readonly panX: number
  readonly panY: number
  readonly split: number
}

export const INITIAL_VIEWPORT: ViewportState = {
  fit: true,
  zoom: 1,
  panX: 0,
  panY: 0,
  split: SPLIT_DEFAULT,
}

/** Measured viewport and the frame it shows; all viewport transitions are judged against it. */
export interface ViewportLayout {
  readonly viewport: Size
  readonly frame: Size
  readonly naturalPerSource: number
}

export interface Point {
  readonly x: number
  readonly y: number
}

/**
 * Zoom in effect: the fitted zoom while `fit` is set (recomputed from the measured viewport, so resizes
 * refit), otherwise the stored zoom. Zero or unmeasured viewports fall back to a zoom of 1.
 */
export function effectiveZoom(state: ViewportState, layout: ViewportLayout): number {
  return state.fit ? fitZoom(layout.viewport, layout.frame, layout.naturalPerSource) : state.zoom
}

/**
 * Stage top-left in viewport pixels. Content that fits is centred regardless of the pan; larger content
 * takes the stored pan clamped so its edges never leave the viewport.
 */
export function stageOffset(state: ViewportState, layout: ViewportLayout): Point {
  const scale = cssScale(effectiveZoom(state, layout), layout.naturalPerSource)
  const stage = {
    width: layout.frame.width * scale,
    height: layout.frame.height * scale,
  }
  return {
    x: placeAxis(state.fit ? 0 : state.panX, layout.viewport.width, stage.width),
    y: placeAxis(state.fit ? 0 : state.panY, layout.viewport.height, stage.height),
  }
}

/**
 * Sets the zoom to `next` (already clamped) while keeping the source point under `anchor` fixed.
 * Returns the same state object when the zoom does not change, so callers can detect a no-op.
 */
function zoomTo(
  state: ViewportState,
  layout: ViewportLayout,
  next: number,
  anchor: Point,
): ViewportState {
  const current = effectiveZoom(state, layout)
  if (next === current) return state
  const offset = stageOffset(state, layout)
  const oldScale = cssScale(current, layout.naturalPerSource)
  const newScale = cssScale(next, layout.naturalPerSource)
  const sourceX = (anchor.x - offset.x) / oldScale
  const sourceY = (anchor.y - offset.y) / oldScale
  const stage = {
    width: layout.frame.width * newScale,
    height: layout.frame.height * newScale,
  }
  return {
    ...state,
    fit: false,
    zoom: next,
    panX: placeAxis(anchor.x - sourceX * newScale, layout.viewport.width, stage.width),
    panY: placeAxis(anchor.y - sourceY * newScale, layout.viewport.height, stage.height),
  }
}

export function zoomBy(
  state: ViewportState,
  layout: ViewportLayout,
  factor: number,
  anchor: Point = viewportCentre(layout),
): ViewportState {
  const next = clamp(effectiveZoom(state, layout) * factor, ZOOM_MIN, ZOOM_MAX)
  return zoomTo(state, layout, next, anchor)
}

/** 100%: one preview pixel per CSS pixel, around the viewport centre. */
export function actualSize(state: ViewportState, layout: ViewportLayout): ViewportState {
  return zoomTo(state, layout, clamp(1, ZOOM_MIN, ZOOM_MAX), viewportCentre(layout))
}

/** Returns to the fitted view and keeps the split position, which belongs to the comparison. */
export function fitView(state: ViewportState): ViewportState {
  if (state.fit && state.panX === 0 && state.panY === 0) return state
  return { ...state, fit: true, panX: 0, panY: 0 }
}

/** Moves the image by a pixel offset, clamped; returns the same state when nothing moves. */
export function panBy(
  state: ViewportState,
  layout: ViewportLayout,
  dx: number,
  dy: number,
): ViewportState {
  const offset = stageOffset(state, layout)
  const scale = cssScale(effectiveZoom(state, layout), layout.naturalPerSource)
  const x = placeAxis(offset.x + dx, layout.viewport.width, layout.frame.width * scale)
  const y = placeAxis(offset.y + dy, layout.viewport.height, layout.frame.height * scale)
  if (x === offset.x && y === offset.y) return state
  return { ...state, fit: false, zoom: effectiveZoom(state, layout), panX: x, panY: y }
}

export function setSplit(state: ViewportState, value: number): ViewportState {
  const share = clamp(Math.round(value), SPLIT_MIN, SPLIT_MAX)
  return share === state.split ? state : { ...state, split: share }
}

/** Applies a keyboard, wheel or button command to the viewport. */
export function applyViewportCommand(
  state: ViewportState,
  layout: ViewportLayout,
  command: ViewportCommand,
  anchor?: Point,
): ViewportState {
  switch (command.kind) {
    case 'zoom':
      return zoomBy(state, layout, command.factor, anchor)
    case 'fit':
      return fitView(state)
    case 'actual':
      return actualSize(state, layout)
    case 'pan':
      return panBy(state, layout, command.dx, command.dy)
  }
}

/** Integer percent shown by the zoom readout; the fitted zoom is labelled as such. */
export function zoomReadout(state: ViewportState, layout: ViewportLayout): string {
  const percent = Math.round(effectiveZoom(state, layout) * 100)
  return state.fit ? `Fit, ${percent}%` : `${percent}%`
}

/**
 * Refit rule: a new source or a change in the frame's dimensions returns to the fitted view with
 * the default split. Settings that keep the dimensions keep zoom, pan and split. Reset also refits
 * because it unmounts the canvas.
 */
export function needsRefit(
  previous: { readonly source: object; readonly dimensions: string },
  next: { readonly source: object; readonly dimensions: string },
): boolean {
  return previous.source !== next.source || previous.dimensions !== next.dimensions
}

function viewportCentre(layout: ViewportLayout): Point {
  return { x: layout.viewport.width / 2, y: layout.viewport.height / 2 }
}
