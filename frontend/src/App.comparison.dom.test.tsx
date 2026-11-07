/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import App from './App'
import { countingUrlApi } from './export/exportTestSupport'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import type { MenuAction, MenuState, NativeMenu } from './processing/nativeMenu'
import type { ExportFormat, ProcessingAdapter } from './processing/types'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

class FakeMenu implements NativeMenu {
  readonly states: MenuState[] = []
  private handler: ((action: MenuAction) => void) | null = null

  setState(state: MenuState): Promise<void> {
    this.states.push(state)
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

interface ExportRequest {
  readonly format: ExportFormat
  readonly settings: string
}

let container: HTMLDivElement
let root: Root | null = null
let controller: StudioController
let menu: FakeMenu
let requests: ExportRequest[]

async function flush(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

let releaseOriginal: () => void = () => undefined

function controllerWithSpy(holdOriginal = false): StudioController {
  const demo = createDemoProcessingAdapter({ urlApi: countingUrlApi() })
  const originalGate = new Promise<void>((resolve) => {
    releaseOriginal = resolve
  })
  const adapter: ProcessingAdapter = {
    ...demo,
    preview: (session, source, settings, context, kind, onProgress) =>
      holdOriginal && kind === 'original'
        ? originalGate.then(() =>
            demo.preview(session, source, settings, context, 'artwork', onProgress),
          )
        : demo.preview(
            session,
            source,
            settings,
            context,
            kind === 'original' ? 'artwork' : kind,
            onProgress,
          ),
    export: (session, source, format, settings, context, onProgress) => {
      requests.push({ format, settings: JSON.stringify(settings) })
      return demo.export(session, source, format, settings, context, onProgress)
    },
  }
  return createStudioController({ adapter })
}

function mount(holdOriginal = false): void {
  menu = new FakeMenu()
  controller = controllerWithSpy(holdOriginal)
  root?.unmount()
  root = createRoot(container)
  act(() => root?.render(<App controller={controller} native={null} menu={menu} />))
}

beforeEach(async () => {
  window.localStorage.setItem('image-to-binary-theme-choice', 'dark')
  container = document.createElement('div')
  document.body.append(container)
  requests = []
  mount()
  await loadDemoThroughPage()
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  controller.dispose()
  container.remove()
  window.localStorage.removeItem('image-to-binary-theme-choice')
})

async function loadDemoThroughPage(): Promise<void> {
  const button = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent?.trim() === 'Load demo',
  )
  if (button === undefined) throw new Error('no Load demo button')
  act(() => button.click())
  await flush()
}

function viewRadio(label: string): HTMLInputElement {
  const found = [...container.querySelectorAll<HTMLLabelElement>('.view-toggle label')].find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  const input = found?.querySelector<HTMLInputElement>('input')
  if (input === null || input === undefined) throw new Error(`no view option ${label}`)
  return input
}

async function choose(label: string): Promise<void> {
  act(() => viewRadio(label).click())
  await flush()
}

function canvasButton(label: string): HTMLButtonElement {
  const found = [
    ...container.querySelectorAll<HTMLButtonElement>('.comparison-canvas button'),
  ].find((candidate) => candidate.textContent?.trim() === label)
  if (found === undefined) throw new Error(`no canvas button ${label}`)
  return found
}

function viewport(): HTMLElement {
  const found = container.querySelector<HTMLElement>('.comparison-viewport')
  if (found === null) throw new Error('no viewport')
  return found
}

function divider(): HTMLElement {
  const found = container.querySelector<HTMLElement>('[role="slider"]')
  if (found === null) throw new Error('no divider')
  return found
}

function press(target: Element, key: string): void {
  act(() => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  })
}

async function exportOnce(): Promise<ExportRequest> {
  const before = requests.length
  await act(async () => {
    await controller.startExport('png').catch(() => undefined)
  })
  await flush()
  const request = requests.at(before)
  if (request === undefined) throw new Error('no export request')
  return request
}

describe('split view through the page', () => {
  it('shows the split with a labelled divider and the Split radio checked', async () => {
    await choose('Split')
    expect(viewRadio('Split').checked).toBe(true)
    expect(divider().getAttribute('aria-valuenow')).toBe('50')
    expect(container.querySelector('h1')?.textContent).toBe('Split comparison')
  })

  it('offers Split beside Original and Artwork in the view choice', async () => {
    const labels = [...container.querySelectorAll<HTMLLabelElement>('.view-toggle label')].map(
      (label) => label.textContent?.trim(),
    )
    expect(labels).toEqual(['Original', 'Artwork', 'Split'])
  })

  it('reports Split to the menu enabled with the artwork gate, and runs it from the menu', async () => {
    expect(menu.states.at(-1)?.split).toBe(true)
    menu.emit('split')
    expect(viewRadio('Split').checked).toBe(true)
  })
})

describe('viewport state never alters the controller or the export', () => {
  it('leaves the snapshot, settings and source metadata unchanged after zoom, pan and split', async () => {
    await choose('Split')
    const snapshotBefore = controller.getSnapshot()
    const metadataBefore = snapshotBefore.metadata
    const settingsBefore = JSON.stringify(snapshotBefore.settings)

    act(() => canvasButton('Zoom in').click())
    act(() => canvasButton('100%').click())
    press(viewport(), 'ArrowLeft')
    press(divider(), 'ArrowRight')
    press(divider(), 'End')

    const snapshotAfter = controller.getSnapshot()
    expect(snapshotAfter.metadata).toBe(metadataBefore)
    expect(JSON.stringify(snapshotAfter.settings)).toBe(settingsBefore)
    expect(snapshotAfter.status).toBe(snapshotBefore.status)
    expect(snapshotAfter.export).toEqual(snapshotBefore.export)
  })

  it('sends a byte-identical export request after zoom, pan and split changes', async () => {
    const first = await exportOnce()
    await choose('Split')
    act(() => canvasButton('Zoom in').click())
    act(() => canvasButton('Zoom in').click())
    press(viewport(), 'ArrowRight')
    press(divider(), 'ArrowLeft')
    const second = await exportOnce()
    expect(second).toEqual(first)
  })

  it('leaves the menu gates unchanged by viewport operations', async () => {
    const before = menu.states.at(-1)
    await choose('Split')
    act(() => canvasButton('Zoom in').click())
    press(divider(), 'Home')
    expect(menu.states.at(-1)).toEqual(before)
  })
})

describe('focus stays inside the page through viewport operations', () => {
  it('keeps focus on the zoom button that was pressed', async () => {
    await choose('Split')
    act(() => canvasButton('Zoom in').focus())
    act(() => canvasButton('Zoom in').click())
    expect(document.activeElement).toBe(canvasButton('Zoom in'))
  })

  it('keeps focus on the divider while it moves', async () => {
    await choose('Split')
    act(() => divider().focus())
    press(divider(), 'ArrowRight')
    expect(document.activeElement).toBe(divider())
  })

  it('moves focus to the heading after a menu split, when focus was on body', async () => {
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    menu.emit('split')
    expect(document.activeElement).not.toBe(document.body)
  })
})

describe('refit on a new source or reset', () => {
  it('returns to the fitted view after a reset and a fresh demo load', async () => {
    await choose('Split')
    act(() => canvasButton('Zoom in').click())
    expect(container.querySelector('.zoom-readout')?.textContent?.startsWith('Fit')).toBe(false)
    act(() => {
      controller.reset()
    })
    await loadDemoThroughPage()
    await choose('Split')
    expect(container.querySelector('.zoom-readout')?.textContent?.startsWith('Fit')).toBe(true)
  })
})

describe('split gate', () => {
  it('disables Split while the ramp draft is invalid, as it does Artwork', async () => {
    act(() => container.querySelector<HTMLInputElement>('input[value="ascii"]')?.click())
    const ramp = container.querySelector<HTMLInputElement>('input[name="ramp"]')
    if (ramp === null) throw new Error('no ramp field')
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    setter?.call(ramp, 'café')
    act(() => {
      ramp.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await flush()
    expect(viewRadio('Split').disabled).toBe(true)
    expect(viewRadio('Artwork').disabled).toBe(true)
    expect(menu.states.at(-1)?.split).toBe(false)
  })
})

describe('settings change keeps the comparison viewport', () => {
  it('keeps zoom and split through a same-size settings change and a re-render', async () => {
    await loadDemoThroughPage()
    await choose('Split')
    press(divider(), 'ArrowRight')
    act(() => canvasButton('Zoom in').click())
    const readoutBefore = container.querySelector('.zoom-readout')?.textContent
    const splitBefore = divider().getAttribute('aria-valuenow')
    act(() => controller.updateSettings({ ...controller.getSnapshot().settings, invert: true }))
    await flush()
    const render = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
      (button) => button.textContent?.trim() === 'Render artwork' && !button.disabled,
    )
    expect(render).toBeDefined()
    act(() => render?.click())
    await flush()
    expect(container.querySelector('.zoom-readout')?.textContent).toBe(readoutBefore)
    expect(divider().getAttribute('aria-valuenow')).toBe(splitBefore)
    expect(viewRadio('Split').checked).toBe(true)
  })
})

describe('split recovers when the original preview arrives late', () => {
  it('requests the artwork once the missing original settles, so Split cannot get stuck', async () => {
    mount(true)
    await loadDemoThroughPage()
    await choose('Split')
    act(() => releaseOriginal())
    for (let index = 0; index < 6; index += 1) await flush()
    expect(container.querySelector('img[alt="Binary artwork preview"]')).not.toBeNull()
    expect(divider().getAttribute('aria-valuenow')).toBe('50')
  })
})

describe('artwork view without an original preview', () => {
  it('shows the artwork once it renders, even when the original preview never arrives', async () => {
    mount(true)
    await loadDemoThroughPage()
    await choose('Artwork')
    expect(container.querySelector('img[alt="Binary artwork preview"]')).not.toBeNull()
    expect(container.querySelector('.empty-canvas')).toBeNull()
  })
})
