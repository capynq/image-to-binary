import { describe, expect, it } from 'vitest'
import {
  PAN_STEP,
  ZOOM_MAX,
  ZOOM_MIN,
  artworkSourceExtent,
  comparisonFrame,
  cssScale,
  fitZoom,
  placeAxis,
  splitKeyValue,
  splitLayout,
  splitValueText,
  viewportKeyCommand,
  wheelCommand,
} from './comparisonGeometry'

/*
 * The sampler (crates/processing/src/sampling.rs `SamplingTask::new`) sets x_scale = W / gw and
 * places cell centres at (k + 0.5) * x_scale, so the artwork spans the whole source on both axes.
 * Cases below are the shapes named in the review: the demo, a source narrower than a cell, a height
 * that is not a multiple of the cell, odd sizes, and artwork pixels whose aspect differs from the source.
 */
describe('artworkSourceExtent', () => {
  it('spans the whole source, so the demo (256 wide, cell 12, artwork 252 px) is 256 wide', () => {
    expect(artworkSourceExtent({ width: 256, height: 192 })).toEqual({ width: 256, height: 192 })
  })

  it('spans a source narrower than one cell without collapsing to zero', () => {
    expect(artworkSourceExtent({ width: 5, height: 9 })).toEqual({ width: 5, height: 9 })
  })

  it('keeps a height that is not a multiple of the cell size', () => {
    expect(artworkSourceExtent({ width: 61, height: 47 })).toEqual({ width: 61, height: 47 })
  })

  it('does not depend on the artwork pixel aspect, which is set by the font-derived cell height', () => {
    const extent = artworkSourceExtent({ width: 64, height: 40 })
    expect(extent).toEqual({ width: 64, height: 40 })
  })

  it('keeps odd sizes exact', () => {
    expect(artworkSourceExtent({ width: 251, height: 191 })).toEqual({ width: 251, height: 191 })
  })
})

describe('comparisonFrame', () => {
  const source = { width: 256, height: 192 }
  const original = { width: 256, height: 192 }
  const artwork = { width: 252, height: 192 }

  it('maps the original view one-to-one onto its own pixels', () => {
    const frame = comparisonFrame({ view: 'original', source, original, artwork })
    expect(frame.frame).toEqual(source)
    expect(frame.naturalPerSource).toBe(1)
  })

  it('sizes the artwork view from its natural pixels, so 100% is one artwork pixel per CSS pixel', () => {
    const frame = comparisonFrame({ view: 'artwork', source, original, artwork })
    expect(frame.frame).toEqual(artwork)
    expect(frame.artworkExtent).toEqual({ width: 256, height: 192 })
    expect(frame.naturalPerSource).toBe(1)
  })

  it('keeps the split on the source frame at the original scale, and stretches the artwork to the source extent', () => {
    const frame = comparisonFrame({ view: 'split', source, original, artwork })
    expect(frame.frame).toEqual({ width: 256, height: 192 })
    expect(frame.artworkExtent).toEqual(frame.originalExtent)
    expect(frame.naturalPerSource).toBe(1)
  })

  it('falls back to the original frame while the artwork is missing', () => {
    const frame = comparisonFrame({ view: 'split', source, original, artwork: null })
    expect(frame.frame).toEqual(source)
    expect(frame.artworkExtent).toEqual(source)
  })

  it('takes the natural scale from the original when no original preview exists', () => {
    const frame = comparisonFrame({ view: 'original', source, original: null, artwork: null })
    expect(frame.naturalPerSource).toBe(1)
  })
})

describe('fitZoom and cssScale', () => {
  it('fits the frame inside the viewport on its tighter axis', () => {
    expect(fitZoom({ width: 500, height: 300 }, { width: 250, height: 250 }, 1)).toBe(1.2)
  })

  it('clamps to the zoom range, so fit never exceeds 16x or drops below the minimum', () => {
    expect(fitZoom({ width: 10_000, height: 10_000 }, { width: 2, height: 2 }, 1)).toBe(ZOOM_MAX)
    expect(fitZoom({ width: 10, height: 10 }, { width: 100_000, height: 100_000 }, 1)).toBe(
      ZOOM_MIN,
    )
  })

  it('returns zoom 1 for an unmeasured viewport, so the first paint is never NaN', () => {
    expect(fitZoom({ width: 0, height: 0 }, { width: 10, height: 10 }, 1)).toBe(1)
  })

  it('scales the natural pixel ratio by zoom', () => {
    expect(cssScale(2, 0.5)).toBe(1)
  })
})

describe('placeAxis', () => {
  it('centres content that fits, ignoring the pan', () => {
    expect(placeAxis(999, 300, 100)).toBe(100)
  })

  it('clamps larger content so it never leaves the viewport', () => {
    expect(placeAxis(50, 300, 800)).toBe(0)
    expect(placeAxis(-900, 300, 800)).toBe(-500)
    expect(placeAxis(-100, 300, 800)).toBe(-100)
  })
})

describe('splitLayout', () => {
  it('gives each image its source extent at one shared scale, so a source coordinate maps to the same CSS position in both', () => {
    const scale = 2
    const layout = splitLayout(
      { width: 256, height: 192 },
      { width: 252, height: 192 },
      { width: 256, height: 192 },
      scale,
      50,
    )
    const sourceX = 126
    const originalCss = sourceX * (layout.original.width / 252)
    const artworkCss = sourceX * (layout.artwork.width / 256)
    expect(originalCss).toBeCloseTo(artworkCss, 9)
    expect(layout.original).toEqual({ width: 252 * scale, height: 192 * scale })
    expect(layout.artwork).toEqual({ width: 256 * scale, height: 192 * scale })
  })

  it('never stretches the artwork to the original size', () => {
    const layout = splitLayout(
      { width: 256, height: 192 },
      { width: 252, height: 192 },
      { width: 256, height: 192 },
      1,
      50,
    )
    expect(layout.artwork.width).toBe(256)
    expect(layout.artwork.width).not.toBe(layout.original.width)
  })

  it('puts the seam at the stated share of the frame width', () => {
    const layout = splitLayout(
      { width: 400, height: 100 },
      { width: 400, height: 100 },
      { width: 400, height: 100 },
      1,
      25,
    )
    expect(layout.seam).toBe(100)
    expect(
      splitLayout(
        { width: 400, height: 100 },
        { width: 400, height: 100 },
        { width: 400, height: 100 },
        1,
        250,
      ).seam,
    ).toBe(400)
  })
})

describe('keyboard and wheel mapping', () => {
  const VIEWPORT = { width: 400, height: 300 }

  it('normalises line and page wheel deltas to pixels before panning', () => {
    const line = wheelCommand(
      { deltaX: 0, deltaY: 3, deltaMode: 1, ctrlKey: false, metaKey: false },
      VIEWPORT,
    )
    expect(line.kind === 'pan' && line.dy).toBe(-48)
    const page = wheelCommand(
      { deltaX: 0, deltaY: 1, deltaMode: 2, ctrlKey: false, metaKey: false },
      VIEWPORT,
    )
    expect(page.kind === 'pan' && page.dy).toBe(-300)
  })
  it('maps the viewport keys to zoom, fit, actual size and pan', () => {
    expect(viewportKeyCommand('+')).toEqual({ kind: 'zoom', factor: 1.25 })
    expect(viewportKeyCommand('-')).toEqual({ kind: 'zoom', factor: 0.8 })
    expect(viewportKeyCommand('0')).toEqual({ kind: 'fit' })
    expect(viewportKeyCommand('1')).toEqual({ kind: 'actual' })
    expect(viewportKeyCommand('ArrowLeft')).toEqual({ kind: 'pan', dx: -PAN_STEP, dy: 0 })
    expect(viewportKeyCommand('ArrowDown')).toEqual({ kind: 'pan', dx: 0, dy: -PAN_STEP })
    expect(viewportKeyCommand('x')).toBeNull()
  })

  it('pans on a plain wheel and zooms only with Ctrl or Cmd, including a trackpad pinch', () => {
    const plain = wheelCommand(
      { deltaX: 0, deltaY: 100, deltaMode: 0, ctrlKey: false, metaKey: false },
      VIEWPORT,
    )
    expect(plain.kind).toBe('pan')
    expect(plain.kind === 'pan' && plain.dy).toBe(-100)
    expect(
      wheelCommand(
        { deltaX: 0, deltaY: -10, deltaMode: 0, ctrlKey: true, metaKey: false },
        VIEWPORT,
      ).kind,
    ).toBe('zoom')
    expect(
      wheelCommand({ deltaX: 0, deltaY: 10, deltaMode: 0, ctrlKey: false, metaKey: true }, VIEWPORT)
        .kind,
    ).toBe('zoom')
  })

  it('zooms in for a negative Ctrl wheel delta and out for a positive one', () => {
    const inward = wheelCommand(
      { deltaX: 0, deltaY: -20, deltaMode: 0, ctrlKey: true, metaKey: false },
      VIEWPORT,
    )
    const outward = wheelCommand(
      { deltaX: 0, deltaY: 20, deltaMode: 0, ctrlKey: true, metaKey: false },
      VIEWPORT,
    )
    expect(inward.kind === 'zoom' && inward.factor > 1).toBe(true)
    expect(outward.kind === 'zoom' && outward.factor < 1).toBe(true)
  })

  it('steps the split with arrows, Shift, Page keys, and jumps with Home and End', () => {
    expect(splitKeyValue(50, 'ArrowLeft', false)).toBe(49)
    expect(splitKeyValue(50, 'ArrowRight', false)).toBe(51)
    expect(splitKeyValue(50, 'ArrowRight', true)).toBe(60)
    expect(splitKeyValue(50, 'PageDown', false)).toBe(40)
    expect(splitKeyValue(50, 'Home', false)).toBe(0)
    expect(splitKeyValue(50, 'End', false)).toBe(100)
    expect(splitKeyValue(0, 'ArrowLeft', false)).toBe(0)
    expect(splitKeyValue(50, 'a', false)).toBeNull()
  })

  it('names both sides of the split for assistive technology', () => {
    expect(splitValueText(30)).toBe('30% original, 70% artwork')
  })
})

describe('per-view box sizing', () => {
  const source = { width: 263, height: 192 }
  const original = { width: 263, height: 192 }
  const artwork = { width: 240, height: 192 }
  const viewport = { width: 400, height: 300 }

  it('fits the artwork view to its natural aspect, so the box matches the bitmap with no stretch', () => {
    const frame = comparisonFrame({ view: 'artwork', source, original, artwork })
    const zoom = fitZoom(viewport, frame.frame, frame.naturalPerSource)
    const scale = cssScale(zoom, frame.naturalPerSource)
    const box = { width: frame.frame.width * scale, height: frame.frame.height * scale }
    expect(box.width / box.height).toBeCloseTo(artwork.width / artwork.height, 12)
    expect(box.width).toBeLessThanOrEqual(viewport.width + 1e-9)
    expect(box.height).toBeLessThanOrEqual(viewport.height + 1e-9)
  })

  it('stretches the split artwork horizontally by (W / gw) / c, and leaves the vertical at 1:1', () => {
    const cell = 24
    const gridWidth = Math.floor(source.width / cell)
    const frame = comparisonFrame({
      view: 'split',
      source,
      original,
      artwork: { width: gridWidth * cell, height: 192 },
    })
    const scale = cssScale(1, frame.naturalPerSource)
    const layout = splitLayout(frame.frame, frame.originalExtent, frame.artworkExtent, scale, 50)
    expect(layout.artwork.width / (gridWidth * cell)).toBeCloseTo(
      source.width / gridWidth / cell,
      12,
    )
    expect(layout.artwork.height / 192).toBeCloseTo(1, 12)
    expect(layout.original.width).toBeCloseTo(layout.artwork.width, 9)
  })
})
