/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PreviewImage, resolutionQuery } from './PreviewImage'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

const NATURAL = { width: 252, height: 192 }

interface FakeQuery {
  readonly text: string
  matches: boolean
  readonly listeners: Set<() => void>
}

/**
 * Evaluates the min/max-resolution pairs this app emits against a ratio, the way a browser would.
 * Queries are shared by text, and only a query whose match state flips fires its listeners.
 */
function matchesRatio(text: string, ratio: number): boolean {
  const min = /min-resolution:\s*([\d.]+)dppx/.exec(text)
  const max = /max-resolution:\s*([\d.]+)dppx/.exec(text)
  if (min === null && max === null) throw new Error(`unsupported media query ${text}`)
  if (min !== null && ratio < Number(min[1])) return false
  if (max !== null && ratio > Number(max[1])) return false
  return true
}

let queries: Map<string, FakeQuery>
let root: Root | null = null
let container: HTMLDivElement
let box: { width: number; height: number }

function setRatio(ratio: number): void {
  Object.defineProperty(window, 'devicePixelRatio', { configurable: true, value: ratio })
  for (const query of queries.values()) {
    const next = matchesRatio(query.text, ratio)
    if (next === query.matches) continue
    query.matches = next
    for (const listener of [...query.listeners]) act(() => listener())
  }
}

function remount(): void {
  if (root !== null) act(() => root?.unmount())
  root = createRoot(container)
  act(() => root?.render(<PreviewImage src="blob:x" alt="preview" width={252} height={192} />))
}

function sampled(): string | null {
  return container.querySelector('img.fixture-preview')?.getAttribute('data-upscaled') ?? null
}

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  queries = new Map()
  Object.defineProperty(window, 'devicePixelRatio', { configurable: true, value: 1 })
  vi.stubGlobal('matchMedia', (text: string) => {
    let query = queries.get(text)
    if (query === undefined) {
      query = {
        text,
        matches: matchesRatio(text, window.devicePixelRatio),
        listeners: new Set(),
      }
      queries.set(text, query)
    }
    const current = query
    return {
      get matches() {
        return current.matches
      },
      addEventListener: (_: string, handler: () => void) => current.listeners.add(handler),
      removeEventListener: (_: string, handler: () => void) => current.listeners.delete(handler),
    }
  })
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(private readonly callback: () => void) {}
      observe(): void {
        this.callback()
      }
      disconnect(): void {
        return
      }
    },
  )
  box = { ...NATURAL }
  vi.spyOn(HTMLImageElement.prototype, 'getBoundingClientRect').mockImplementation(
    () =>
      ({
        width: box.width,
        height: box.height,
        top: 0,
        left: 0,
        right: box.width,
        bottom: box.height,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      }) as DOMRect,
  )
  root = createRoot(container)
  act(() => root?.render(<PreviewImage src="blob:x" alt="preview" width={252} height={192} />))
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('PreviewImage device pixel ratio', () => {
  it('flips to pixelated sampling at 3x but not at 2x for a preview between one half and one times its size', () => {
    box = { width: 101, height: 77 }
    remount()
    setRatio(2)
    expect(sampled()).toBeNull()
    setRatio(3)
    expect(sampled()).toBe('true')
  })

  it('re-arms after each change, so a later move back to 1x is also seen', () => {
    box = { width: 101, height: 77 }
    remount()
    setRatio(2)
    setRatio(3)
    expect(sampled()).toBe('true')
    setRatio(1)
    expect(sampled()).toBeNull()
  })

  it('transitions for a fractional ratio such as 1.7999999523', () => {
    setRatio(1.7999999523)
    expect(sampled()).toBe('true')
    setRatio(1)
    expect(sampled()).toBeNull()
  })

  it('builds exact min and max resolution queries', () => {
    expect(resolutionQuery(2)).toBe('(min-resolution: 2dppx) and (max-resolution: 2dppx)')
  })
})
