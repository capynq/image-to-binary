import { describe, expect, it } from 'vitest'
import { ZOOM_MAX, ZOOM_MIN } from './comparisonGeometry'
import {
  INITIAL_VIEWPORT,
  actualSize,
  applyViewportCommand,
  effectiveZoom,
  fitView,
  needsRefit,
  panBy,
  setSplit,
  stageOffset,
  zoomBy,
  zoomReadout,
} from './comparisonViewport'
import type { ViewportLayout, ViewportState } from './comparisonViewport'

const LAYOUT: ViewportLayout = {
  viewport: { width: 400, height: 300 },
  frame: { width: 1000, height: 500 },
  naturalPerSource: 1,
}

const FIT: ViewportState = INITIAL_VIEWPORT

describe('fit', () => {
  it('starts fitted with the default split and no pan', () => {
    expect(FIT.fit).toBe(true)
    expect(FIT.split).toBe(50)
    expect(effectiveZoom(FIT, LAYOUT)).toBeCloseTo(0.4)
  })

  it('centres the fitted stage inside the viewport', () => {
    const offset = stageOffset(FIT, LAYOUT)
    expect(offset.x).toBe(0)
    expect(offset.y).toBeCloseTo((300 - 500 * 0.4) / 2)
  })

  it('returns to fit and clears the pan but keeps the split', () => {
    const zoomed = { ...FIT, fit: false, zoom: 2, panX: -50, panY: -20, split: 30 }
    const back = fitView(zoomed)
    expect(back.fit).toBe(true)
    expect(back.panX).toBe(0)
    expect(back.panY).toBe(0)
    expect(back.split).toBe(30)
  })
})

describe('zoom', () => {
  it('clamps to the maximum, so repeated zoom in stops at 16x of natural size', () => {
    let state = FIT
    for (let index = 0; index < 60; index += 1) state = zoomBy(state, LAYOUT, 1.25)
    expect(effectiveZoom(state, LAYOUT)).toBe(ZOOM_MAX)
  })

  it('clamps to the minimum, so repeated zoom out cannot lose the image', () => {
    let state = FIT
    for (let index = 0; index < 60; index += 1) state = zoomBy(state, LAYOUT, 0.8)
    expect(effectiveZoom(state, LAYOUT)).toBe(ZOOM_MIN)
  })

  it('returns the same state object at a limit, so a no-op does not re-render', () => {
    const max = { ...FIT, fit: false, zoom: ZOOM_MAX }
    expect(zoomBy(max, LAYOUT, 1.25)).toBe(max)
  })

  it('keeps the point under the anchor fixed while zooming, away from the pan limits', () => {
    const start = actualSize(FIT, LAYOUT)
    const anchor = { x: 200, y: 150 }
    const before = stageOffset(start, LAYOUT)
    const scaleBefore = effectiveZoom(start, LAYOUT)
    const sourceX = (anchor.x - before.x) / scaleBefore
    const sourceY = (anchor.y - before.y) / scaleBefore
    const next = zoomBy(start, LAYOUT, 1.25, anchor)
    const after = stageOffset(next, LAYOUT)
    const scale = effectiveZoom(next, LAYOUT)
    expect(after.x + sourceX * scale).toBeCloseTo(anchor.x, 6)
    expect(after.y + sourceY * scale).toBeCloseTo(anchor.y, 6)
  })

  it('clamps the anchored offset at the edge rather than showing empty space', () => {
    const next = zoomBy(FIT, LAYOUT, 2, { x: 100, y: 80 })
    const after = stageOffset(next, LAYOUT)
    expect(after.y).toBe(0)
  })

  it('sets 100% as one natural pixel per CSS pixel', () => {
    const state = actualSize(FIT, LAYOUT)
    expect(effectiveZoom(state, LAYOUT)).toBe(1)
    expect(zoomReadout(state, LAYOUT)).toBe('100%')
  })

  it('labels the fitted zoom in the readout', () => {
    expect(zoomReadout(FIT, LAYOUT)).toBe('Fit, 40%')
  })
})

describe('pan', () => {
  const zoomed = actualSize(FIT, LAYOUT)

  it('moves a zoomed image and clamps it so the image stays within reach', () => {
    const moved = panBy(zoomed, LAYOUT, 10_000, 10_000)
    const offset = stageOffset(moved, LAYOUT)
    expect(offset.x).toBe(0)
    expect(offset.y).toBe(0)
    const far = panBy(zoomed, LAYOUT, -10_000, -10_000)
    const farOffset = stageOffset(far, LAYOUT)
    expect(farOffset.x).toBe(400 - 1000)
    expect(farOffset.y).toBe(300 - 500)
  })

  it('returns the same state when nothing moves, so the wheel can pass scroll through', () => {
    expect(panBy(FIT, LAYOUT, 40, 40)).toBe(FIT)
    const atEdge = panBy(zoomed, LAYOUT, 10_000, 0)
    expect(panBy(atEdge, LAYOUT, 40, 0)).toBe(atEdge)
  })

  it('leaves fit when the image really moves', () => {
    const scaled = { ...FIT, fit: false, zoom: 2 }
    const moved = panBy(scaled, LAYOUT, -40, 0)
    expect(moved.fit).toBe(false)
  })
})

describe('split and commands', () => {
  it('clamps and rounds the split share', () => {
    expect(setSplit(FIT, 140).split).toBe(100)
    expect(setSplit(FIT, -5).split).toBe(0)
    expect(setSplit(FIT, 33.6).split).toBe(34)
  })

  it('returns the same state when the split does not change', () => {
    expect(setSplit(FIT, 50)).toBe(FIT)
  })

  it('applies a fit command through the same reducer as the buttons', () => {
    const zoomed = applyViewportCommand(FIT, LAYOUT, { kind: 'actual' })
    expect(applyViewportCommand(zoomed, LAYOUT, { kind: 'fit' }).fit).toBe(true)
  })
})

describe('refit rule', () => {
  const source = { name: 'a' }
  const other = { name: 'b' }

  it('refits when the source object changes', () => {
    expect(
      needsRefit({ source, dimensions: '10x10' }, { source: other, dimensions: '10x10' }),
    ).toBe(true)
  })

  it('refits when the frame dimensions change, as a cell-size change does', () => {
    expect(needsRefit({ source, dimensions: '10x10' }, { source, dimensions: '12x10' })).toBe(true)
  })

  it('keeps the view when the source and the dimensions are unchanged, as a same-size settings change does', () => {
    expect(needsRefit({ source, dimensions: '10x10' }, { source, dimensions: '10x10' })).toBe(false)
  })
})
