/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ComparisonCanvas } from './ComparisonCanvas'
import type { ComparisonCanvasProps } from './ComparisonCanvas'
import type { PreviewSnapshot } from './processing/types'
import type { FramePlayback } from './useFrameClock'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) {
    const mounted = root
    root = null
    act(() => mounted.unmount())
  }
  container.remove()
})

const FRAMES = 4

function frameUrls(kind: string): string[] {
  return Array.from({ length: FRAMES }, (_, index) => `blob:${kind}-${index}`)
}

function snapshot(kind: string): PreviewSnapshot {
  return { url: `blob:${kind}-0`, width: 8, height: 8, frames: frameUrls(kind) }
}

function playback(overrides: Partial<FramePlayback> = {}): FramePlayback {
  return {
    frameCount: FRAMES,
    frame: 0,
    playing: false,
    setFrame: vi.fn(),
    setPlaying: vi.fn(),
    ...overrides,
  }
}

function props(overrides: Partial<ComparisonCanvasProps> = {}): ComparisonCanvasProps {
  return {
    view: 'split',
    source: { width: 8, height: 8 },
    sourceToken: { name: 'animated' },
    original: snapshot('original'),
    artwork: snapshot('artwork'),
    originalAlt: 'Original image',
    artworkAlt: 'Binary artwork preview',
    ...overrides,
  }
}

function show(element: React.ReactElement): void {
  root ??= createRoot(container)
  const target = root
  act(() => target.render(element))
}

function images(): HTMLImageElement[] {
  return Array.from(container.querySelectorAll<HTMLImageElement>('img'))
}

function sources(): string[] {
  return images().map((image) => image.getAttribute('src') ?? '')
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('button')).find(
    (candidate) => candidate.textContent === label,
  )
  if (found === undefined) throw new Error(`no ${label} button`)
  return found
}

function scrubber(): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>('input[type="range"][aria-label="Frame"]')
  if (found === null) throw new Error('no frame scrubber')
  return found
}

describe('ComparisonCanvas playback controls', () => {
  it('renders no playback controls for a still source', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork', playback: null })} />)
    expect(container.querySelector('.playback-controls')).toBeNull()
  })

  it('labels the toggle Play when paused and Pause when playing', () => {
    const setPlaying = vi.fn()
    show(<ComparisonCanvas {...props({ view: 'artwork', playback: playback({ setPlaying }) })} />)
    act(() => button('Play').click())
    expect(setPlaying).toHaveBeenLastCalledWith(true)

    show(
      <ComparisonCanvas
        {...props({ view: 'artwork', playback: playback({ playing: true, setPlaying }) })}
      />,
    )
    act(() => button('Pause').click())
    expect(setPlaying).toHaveBeenLastCalledWith(false)
  })

  it('exposes a labelled scrubber with one-based value text', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork', playback: playback({ frame: 2 }) })} />)
    const input = scrubber()
    expect(input.min).toBe('0')
    expect(input.max).toBe(String(FRAMES - 1))
    expect(input.value).toBe('2')
    expect(input.getAttribute('aria-valuetext')).toBe(`Frame 3 of ${FRAMES}`)
    expect(container.querySelector('.frame-readout')?.textContent).toBe(`Frame 3 of ${FRAMES}`)
  })

  it('sends the scrubbed frame to the clock', () => {
    const setFrame = vi.fn()
    show(<ComparisonCanvas {...props({ view: 'artwork', playback: playback({ setFrame }) })} />)
    const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    act(() => {
      valueSetter?.call(scrubber(), '3')
      scrubber().dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(setFrame).toHaveBeenCalledWith(3)
  })

  it('shows the same frame index in both images of the split view', () => {
    show(<ComparisonCanvas {...props({ view: 'split', playback: playback({ frame: 2 }) })} />)
    expect(sources()).toEqual(['blob:original-2', 'blob:artwork-2'])
  })

  it('shows the frame in the original and artwork views', () => {
    show(<ComparisonCanvas {...props({ view: 'original', playback: playback({ frame: 1 }) })} />)
    expect(sources()).toEqual(['blob:original-1'])
    show(<ComparisonCanvas {...props({ view: 'artwork', playback: playback({ frame: 3 }) })} />)
    expect(sources()).toEqual(['blob:artwork-3'])
  })

  it('falls back to the start image for a still preview with no frame list', () => {
    show(
      <ComparisonCanvas
        {...props({
          view: 'artwork',
          artwork: { url: 'blob:still', width: 8, height: 8 },
          playback: playback({ frame: 2 }),
        })}
      />,
    )
    expect(sources()).toEqual(['blob:still'])
  })
})
