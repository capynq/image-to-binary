/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StudioAnimation } from './processing/types'
import { useDecodedFrames, useFrameClock } from './useFrameClock'
import type { FrameClock } from './useFrameClock'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

interface Clock {
  now: number
  nextId: number
  callbacks: Map<number, FrameRequestCallback>
}

let clock: Clock
let container: HTMLDivElement
let root: Root | null = null
let latest: FrameClock
let hidden = false
let reduceMotion = false

/** Runs one animation frame at `ms` later than the previous one. */
function advance(ms: number): void {
  clock.now += ms
  const pending = [...clock.callbacks.values()]
  clock.callbacks.clear()
  act(() => {
    for (const callback of pending) callback(clock.now)
  })
}

function setHidden(next: boolean): void {
  hidden = next
  act(() => {
    document.dispatchEvent(new Event('visibilitychange'))
  })
}

interface HarnessProps {
  readonly animation: StudioAnimation | null
  readonly identity: object | null
  readonly ready?: boolean
}

function Harness({ animation, identity, ready }: HarnessProps) {
  const value = useFrameClock(
    ready === undefined ? { animation, identity } : { animation, identity, ready },
  )
  latest = value
  return null
}

function render(props: HarnessProps): void {
  root ??= createRoot(container)
  const target = root
  act(() => target.render(<Harness {...props} />))
}

function animation(delaysCs: number[], loopCount = 0): StudioAnimation {
  return { frameCount: delaysCs.length, delaysCs, loopCount }
}

const SOURCE = { name: 'source' }

beforeEach(() => {
  clock = { now: 0, nextId: 1, callbacks: new Map() }
  hidden = false
  reduceMotion = false
  container = document.createElement('div')
  document.body.append(container)
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = clock.nextId
    clock.nextId += 1
    clock.callbacks.set(id, callback)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => {
    clock.callbacks.delete(id)
  })
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('prefers-reduced-motion') && reduceMotion,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  }))
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden })
})

afterEach(() => {
  if (root !== null) {
    const mounted = root
    root = null
    act(() => mounted.unmount())
  }
  container.remove()
  vi.unstubAllGlobals()
  delete (document as { hidden?: boolean }).hidden
})

describe('useFrameClock timing', () => {
  it('advances each frame after its own delay in centiseconds', () => {
    const source = animation([5, 10, 2])
    render({ animation: source, identity: SOURCE })
    advance(0)
    expect(latest.frame).toBe(0)
    advance(49)
    expect(latest.frame).toBe(0)
    advance(1)
    expect(latest.frame).toBe(1)
    advance(100)
    expect(latest.frame).toBe(2)
    expect(latest.playing).toBe(true)
  })

  it('accumulates late callbacks instead of advancing one frame per callback', () => {
    render({ animation: animation([5, 5, 5]), identity: SOURCE })
    advance(0)
    advance(120)
    expect(latest.frame).toBe(2)
  })

  it('clamps delays shorter than 10 ms so the clock always moves', () => {
    render({ animation: animation([0, 0]), identity: SOURCE })
    advance(0)
    advance(9)
    expect(latest.frame).toBe(0)
    advance(1)
    expect(latest.frame).toBe(1)
  })

  it('stops on the last frame once the single play ends when loopCount is 0', () => {
    render({ animation: animation([1, 1], 0), identity: SOURCE })
    advance(0)
    advance(10)
    expect(latest.frame).toBe(1)
    advance(10)
    expect(latest.frame).toBe(1)
    expect(latest.playing).toBe(false)
    expect(clock.callbacks.size).toBe(0)
  })

  it('plays n extra times for a finite loop count and then stops on the last frame', () => {
    render({ animation: animation([1, 1], 2), identity: SOURCE })
    advance(0)
    const seen: number[] = []
    for (let step = 0; step < 6; step += 1) {
      advance(10)
      seen.push(latest.frame)
    }
    expect(seen).toEqual([1, 0, 1, 0, 1, 1])
    expect(latest.playing).toBe(false)
  })

  it('loops forever for an infinite loop count', () => {
    render({ animation: animation([1, 1], Number.POSITIVE_INFINITY), identity: SOURCE })
    advance(0)
    const seen: number[] = []
    for (let step = 0; step < 8; step += 1) {
      advance(10)
      seen.push(latest.frame)
    }
    expect(seen).toEqual([1, 0, 1, 0, 1, 0, 1, 0])
    expect(latest.playing).toBe(true)
  })

  it('shows frame 0 and never schedules a frame without an animation', () => {
    render({ animation: null, identity: SOURCE })
    advance(0)
    expect(latest.frame).toBe(0)
    expect(latest.playing).toBe(false)
    expect(clock.callbacks.size).toBe(0)
  })
})

describe('useFrameClock visibility and readiness', () => {
  it('pauses while the document is hidden and resumes without a catch-up jump', () => {
    render({ animation: animation([5, 5, 5]), identity: SOURCE })
    advance(0)
    advance(50)
    expect(latest.frame).toBe(1)
    setHidden(true)
    expect(clock.callbacks.size).toBe(0)
    advance(10_000)
    expect(latest.frame).toBe(1)
    setHidden(false)
    advance(0)
    expect(latest.frame).toBe(1)
    advance(49)
    expect(latest.frame).toBe(1)
    advance(1)
    expect(latest.frame).toBe(2)
    expect(latest.playing).toBe(true)
  })

  it('holds the frame until the frames are ready', () => {
    render({ animation: animation([5, 5]), identity: SOURCE, ready: false })
    advance(0)
    advance(500)
    expect(latest.frame).toBe(0)
    expect(clock.callbacks.size).toBe(0)
    render({ animation: animation([5, 5]), identity: SOURCE, ready: true })
    advance(0)
    advance(50)
    expect(latest.frame).toBe(1)
  })

  it('cancels the pending frame on unmount', () => {
    render({ animation: animation([5, 5]), identity: SOURCE })
    expect(clock.callbacks.size).toBe(1)
    act(() => root?.unmount())
    root = null
    expect(clock.callbacks.size).toBe(0)
  })
})

describe('useFrameClock controls', () => {
  it('starts paused on frame 0 under reduced motion', () => {
    reduceMotion = true
    render({ animation: animation([5, 5]), identity: SOURCE })
    advance(0)
    advance(500)
    expect(latest.frame).toBe(0)
    expect(latest.playing).toBe(false)
    expect(clock.callbacks.size).toBe(0)
    act(() => latest.setPlaying(true))
    advance(0)
    advance(50)
    expect(latest.frame).toBe(1)
  })

  it('setFrame shows the frame, clamps it and pauses playback', () => {
    render({ animation: animation([5, 5, 5]), identity: SOURCE })
    advance(0)
    act(() => latest.setFrame(2))
    expect(latest.frame).toBe(2)
    expect(latest.playing).toBe(false)
    act(() => latest.setFrame(99))
    expect(latest.frame).toBe(2)
    act(() => latest.setFrame(-4))
    expect(latest.frame).toBe(0)
    advance(100)
    expect(latest.frame).toBe(0)
  })

  it('setPlaying(true) on the final frame restarts from frame 0', () => {
    render({ animation: animation([1, 1], 0), identity: SOURCE })
    advance(0)
    advance(20)
    expect(latest.playing).toBe(false)
    expect(latest.frame).toBe(1)
    act(() => latest.setPlaying(true))
    expect(latest.frame).toBe(0)
    expect(latest.playing).toBe(true)
  })

  it('resets to frame 0 for a new animation identity', () => {
    render({ animation: animation([5, 5, 5]), identity: SOURCE })
    advance(0)
    advance(60)
    expect(latest.frame).toBe(1)
    act(() => latest.setFrame(2))
    render({ animation: animation([5, 5, 5]), identity: { name: 'next' } })
    expect(latest.frame).toBe(0)
    expect(latest.playing).toBe(true)
  })

  it('resets paused for a new identity under reduced motion', () => {
    reduceMotion = true
    render({ animation: animation([5, 5]), identity: SOURCE })
    act(() => latest.setPlaying(true))
    render({ animation: animation([5, 5]), identity: { name: 'next' } })
    expect(latest.playing).toBe(false)
    expect(latest.frame).toBe(0)
  })
})

describe('useDecodedFrames', () => {
  function Decoding({
    first,
    second,
    onReady,
  }: {
    first?: string[]
    second?: string[]
    onReady: (ready: boolean) => void
  }) {
    const ready = useDecodedFrames(first, second)
    onReady(ready)
    return null
  }

  it('reports ready at once when there are no frames', () => {
    let ready = false
    act(() => {
      createRoot(container).render(<Decoding onReady={(value) => (ready = value)} />)
    })
    expect(ready).toBe(true)
  })

  it('decodes each frame in order before reporting ready', async () => {
    const decoded: string[] = []
    class FakeImage {
      src = ''
      decode(): Promise<void> {
        decoded.push(this.src)
        return Promise.resolve()
      }
    }
    vi.stubGlobal('Image', FakeImage)
    const frames = ['blob:a-0', 'blob:a-1']
    let ready = false
    act(() => {
      createRoot(container).render(<Decoding first={frames} onReady={(value) => (ready = value)} />)
    })
    expect(ready).toBe(false)
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(decoded).toEqual(frames)
    expect(ready).toBe(true)
  })

  it('counts a frame whose decode fails as decoded', async () => {
    class RejectingImage {
      src = ''
      decode(): Promise<void> {
        return Promise.reject(new Error('revoked'))
      }
    }
    vi.stubGlobal('Image', RejectingImage)
    let ready = false
    act(() => {
      createRoot(container).render(
        <Decoding second={['blob:gone-0']} onReady={(value) => (ready = value)} />,
      )
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(ready).toBe(true)
  })
})
