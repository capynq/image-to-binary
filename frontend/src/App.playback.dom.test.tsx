/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import type { MenuAction, MenuState, NativeMenu } from './processing/nativeMenu'
import { animatedTestAdapter } from './processing/animatedTestAdapter'
import type { AnimatedTestAdapter } from './processing/animatedTestAdapter'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

class FakeMenu implements NativeMenu {
  private handler: ((action: MenuAction) => void) | null = null

  setState(_state: MenuState): Promise<void> {
    return Promise.resolve()
  }

  listen(handler: (action: MenuAction) => void): Promise<() => void> {
    this.handler = handler
    return Promise.resolve(() => {
      this.handler = null
    })
  }

  emit(action: MenuAction): void {
    act(() => this.handler?.(action))
  }
}

let container: HTMLDivElement
let root: Root | null = null
let fake: AnimatedTestAdapter
let controller: StudioController
let menu: FakeMenu

async function settle(milliseconds = 0): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, milliseconds))
  })
}

function scrubberInput(): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>('input[type="range"][aria-label="Frame"]')
  if (found === null) throw new Error('no frame scrubber')
  return found
}

function scrubTo(frame: number): void {
  const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  act(() => {
    valueSetter?.call(scrubberInput(), String(frame))
    scrubberInput().dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function lastPreview(): AnimatedTestAdapter['previews'][number] | undefined {
  return fake.previews.at(-1)
}

beforeEach(() => {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('reduce'),
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  }))
  container = document.createElement('div')
  document.body.append(container)
  fake = animatedTestAdapter({ frameCount: 4 })
  controller = createStudioController({ adapter: fake.adapter })
  menu = new FakeMenu()
})

afterEach(() => {
  if (root !== null) {
    const mounted = root
    root = null
    act(() => mounted.unmount())
  }
  container.remove()
  vi.unstubAllGlobals()
})

async function mountAnimated(): Promise<void> {
  root = createRoot(container)
  act(() => root?.render(<App controller={controller} native={null} menu={menu} />))
  await act(async () => {
    await controller.import({ kind: 'demo' })
  })
  await settle()
  menu.emit('artwork')
  await settle()
}

describe('App animation playback wiring', () => {
  it('shows the playback controls for an animated import and starts paused under reduced motion', async () => {
    await mountAnimated()
    expect(scrubberInput().max).toBe('3')
    const toggle = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent === 'Play',
    )
    expect(toggle).toBeDefined()
  })

  it('scrubbing pauses playback and the auto-render uses the scrubbed frame as its start frame', async () => {
    await mountAnimated()
    scrubTo(2)
    expect(scrubberInput().value).toBe('2')
    expect(scrubberInput().getAttribute('aria-valuetext')).toBe('Frame 3 of 4')

    const before = fake.previews.length
    act(() => controller.updateSettings({ ...controller.getSnapshot().settings, invert: true }))
    await settle(300)
    expect(fake.previews.length).toBe(before + 1)
    expect(lastPreview()).toEqual({ kind: 'artwork', options: { startFrame: 2 } })
  })

  it('a manual render starts on the shown frame', async () => {
    await mountAnimated()
    scrubTo(1)
    menu.emit('render')
    await settle()
    expect(lastPreview()).toEqual({ kind: 'artwork', options: { startFrame: 1 } })
  })

  it('switching to the artwork view starts the render on the shown frame', async () => {
    await mountAnimated()
    scrubTo(3)
    menu.emit('split')
    await settle()
    expect(lastPreview()).toEqual({ kind: 'artwork', options: { startFrame: 3 } })
  })

  it('exports the shown frame', async () => {
    await mountAnimated()
    scrubTo(2)
    menu.emit('export')
    await settle()
    expect(fake.exports.at(-1)).toEqual({ format: 'png', options: { frame: 2 } })
  })
})
