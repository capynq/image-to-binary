/** Views of the comparison canvas. `split` is UI-only and never reaches the controller. */
export type ViewMode = 'original' | 'artwork' | 'split'

export interface Size {
  readonly width: number
  readonly height: number
}

export const ZOOM_MIN = 1 / 32
export const ZOOM_MAX = 16
export const ZOOM_STEP = 1.25
export const PAN_STEP = 40
export const SPLIT_MIN = 0
export const SPLIT_MAX = 100
export const SPLIT_DEFAULT = 50

const WHEEL_ZOOM_RATE = 0.01
const WHEEL_DELTA_LIMIT = 100

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value))
}

/**
 * Source-space extent of the artwork in a split. The sampler places cell centres at `(k + 0.5) * W / gw`
 * (and likewise vertically), so the artwork spans the whole source on both axes, whatever the cell size,
 * the grid count or the font-derived cell height. Classic cells are drawn `c` pixels wide but span
 * `W / gw` source units, so the split draws the artwork stretched by `(W / gw) / c` horizontally; that
 * stretch is the linear mapping, not distortion. Average sampling compresses only its last partial cell
 * into the remaining source, a difference of at most one cell at the far edge, which is not modelled here.
 */
export function artworkSourceExtent(source: Size): Size {
  return { width: source.width, height: source.height }
}

/** The canvas content in its units, the scale that maps preview pixels to it, and each image's extent. */
export interface ComparisonFrame {
  /**
   * Source units for the original and split; the artwork's own pixels for the artwork view, so the
   * artwork is shown at its natural aspect and matches the PNG export.
   */
  readonly frame: Size
  readonly originalExtent: Size
  readonly artworkExtent: Size
  /** Preview pixels per source unit of the reference image: at zoom 1 one pixel maps to one CSS pixel. */
  readonly naturalPerSource: number
}

export interface FrameInput {
  readonly view: ViewMode
  readonly source: Size
  readonly original: Size | null
  readonly artwork: Size | null
}

/**
 * Builds the frame for a view. A split maps both images over one source frame at the original's
 * natural scale (so the original is 1:1 at 100%), with the artwork stretched to the source extent.
 * The artwork view is sized from its natural pixels (one CSS px per pixel at 100%, no stretch).
 * The original view uses its own pixels.
 */
export function comparisonFrame({ view, source, original, artwork }: FrameInput): ComparisonFrame {
  const originalExtent = source
  const artworkExtent = artworkSourceExtent(source)
  if (view === 'original' || artwork === null) {
    return {
      frame: originalExtent,
      originalExtent,
      artworkExtent,
      naturalPerSource: original === null ? 1 : ratio(original.width, originalExtent.width),
    }
  }
  if (view === 'artwork') {
    return {
      frame: { width: artwork.width, height: artwork.height },
      originalExtent,
      artworkExtent,
      naturalPerSource: 1,
    }
  }
  return {
    frame: {
      width: Math.max(originalExtent.width, artworkExtent.width),
      height: Math.max(originalExtent.height, artworkExtent.height),
    },
    originalExtent,
    artworkExtent,
    naturalPerSource:
      original === null
        ? ratio(artwork.width, artworkExtent.width)
        : ratio(original.width, originalExtent.width),
  }
}

function ratio(pixels: number, units: number): number {
  return units > 0 && pixels > 0 ? pixels / units : 1
}

/** Zoom that fits the frame inside the viewport, kept within the zoom range. */
export function fitZoom(viewport: Size, frame: Size, naturalPerSource: number): number {
  if (viewport.width <= 0 || viewport.height <= 0 || frame.width <= 0 || frame.height <= 0) {
    return 1
  }
  const scale = Math.min(viewport.width / frame.width, viewport.height / frame.height)
  return clamp(scale / naturalPerSource, ZOOM_MIN, ZOOM_MAX)
}

/** CSS pixels per source unit at a zoom. */
export function cssScale(zoom: number, naturalPerSource: number): number {
  return zoom * naturalPerSource
}

/**
 * Positions one axis of the stage. Content that fits is centred and ignores the pan; larger
 * content is clamped so its edges never leave the viewport, which keeps the image in reach.
 */
export function placeAxis(offset: number, viewport: number, content: number): number {
  if (content <= viewport) return (viewport - content) / 2
  return clamp(offset, viewport - content, 0)
}

/**
 * Splits the aligned layout for a split view. Both images start at the stage origin and share one
 * scale, so a source coordinate lands at the same CSS position in each; the seam sits at the given
 * share of the frame. Each image is sized to its source extent from `artworkSourceExtent`, so the
 * artwork covers the same source region as the original.
 */
export interface SplitLayout {
  readonly stage: Size
  readonly seam: number
  readonly original: Size
  readonly artwork: Size
}

export function splitLayout(
  frame: Size,
  originalExtent: Size,
  artworkExtent: Size,
  scale: number,
  split: number,
): SplitLayout {
  const stage = { width: frame.width * scale, height: frame.height * scale }
  return {
    stage,
    seam: (stage.width * clamp(split, SPLIT_MIN, SPLIT_MAX)) / SPLIT_MAX,
    original: { width: originalExtent.width * scale, height: originalExtent.height * scale },
    artwork: { width: artworkExtent.width * scale, height: artworkExtent.height * scale },
  }
}

export type ViewportCommand =
  | { readonly kind: 'zoom'; readonly factor: number }
  | { readonly kind: 'fit' }
  | { readonly kind: 'actual' }
  | { readonly kind: 'pan'; readonly dx: number; readonly dy: number }

/** Keys handled while the viewport itself has focus. Arrow keys move the image in their direction. */
export function viewportKeyCommand(key: string): ViewportCommand | null {
  switch (key) {
    case '+':
    case '=':
      return { kind: 'zoom', factor: ZOOM_STEP }
    case '-':
    case '_':
      return { kind: 'zoom', factor: 1 / ZOOM_STEP }
    case '0':
      return { kind: 'fit' }
    case '1':
      return { kind: 'actual' }
    case 'ArrowLeft':
      return { kind: 'pan', dx: -PAN_STEP, dy: 0 }
    case 'ArrowRight':
      return { kind: 'pan', dx: PAN_STEP, dy: 0 }
    case 'ArrowUp':
      return { kind: 'pan', dx: 0, dy: PAN_STEP }
    case 'ArrowDown':
      return { kind: 'pan', dx: 0, dy: -PAN_STEP }
    default:
      return null
  }
}

export type WheelCommand =
  | { readonly kind: 'zoom'; readonly factor: number }
  | { readonly kind: 'pan'; readonly dx: number; readonly dy: number }

/** Pixels per wheel line; browsers report line-mode wheels in lines, as Chromium does on some platforms. */
const WHEEL_LINE_PX = 16

/**
 * Plain wheel pans the image and Ctrl/Cmd wheel (including a trackpad pinch, which browsers report
 * as a Ctrl wheel) zooms. Deltas are normalised to pixels first: lines use a fixed pixel size and
 * pages use the viewport height. The caller decides whether to suppress page scroll from the result.
 */
export function wheelCommand(
  event: {
    readonly deltaX: number
    readonly deltaY: number
    readonly deltaMode: number
    readonly ctrlKey: boolean
    readonly metaKey: boolean
  },
  viewport: Size,
): WheelCommand {
  const unit = event.deltaMode === 1 ? WHEEL_LINE_PX : event.deltaMode === 2 ? viewport.height : 1
  const deltaX = event.deltaX * unit
  const deltaY = event.deltaY * unit
  if (event.ctrlKey || event.metaKey) {
    const delta = clamp(deltaY, -WHEEL_DELTA_LIMIT, WHEEL_DELTA_LIMIT)
    return { kind: 'zoom', factor: Math.exp(-delta * WHEEL_ZOOM_RATE) }
  }
  return { kind: 'pan', dx: -deltaX, dy: -deltaY }
}

/**
 * New split share for a keyboard key on the divider: arrows step by one percent (ten with Shift),
 * PageUp and PageDown by ten, Home and End jump to the ends. Returns null for other keys.
 */
export function splitKeyValue(current: number, key: string, shift: boolean): number | null {
  const step = shift ? 10 : 1
  switch (key) {
    case 'ArrowLeft':
    case 'ArrowDown':
      return clamp(current - step, SPLIT_MIN, SPLIT_MAX)
    case 'ArrowRight':
    case 'ArrowUp':
      return clamp(current + step, SPLIT_MIN, SPLIT_MAX)
    case 'PageDown':
      return clamp(current - 10, SPLIT_MIN, SPLIT_MAX)
    case 'PageUp':
      return clamp(current + 10, SPLIT_MIN, SPLIT_MAX)
    case 'Home':
      return SPLIT_MIN
    case 'End':
      return SPLIT_MAX
    default:
      return null
  }
}

/** Text describing the current split for `aria-valuetext`, naming which image sits on each side. */
export function splitValueText(split: number): string {
  const share = Math.round(split)
  return `${share}% original, ${SPLIT_MAX - share}% artwork`
}
