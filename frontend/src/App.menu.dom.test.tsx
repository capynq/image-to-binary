/** @vitest-environment jsdom */
import { StrictMode, act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { countingUrlApi } from './export/exportTestSupport'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import type { MenuAction, MenuState, NativeMenu } from './processing/nativeMenu'
import { ExportCancelledError } from './processing/types'
import type { ProcessingAdapter } from './processing/types'
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

  get last(): MenuState {
    const state = this.states.at(-1)
    if (state === undefined) throw new Error('no menu state was sent')
    return state
  }
}

let container: HTMLDivElement
let root: Root | null = null
let controller: StudioController
let menu: FakeMenu
let exportRequests: number

async function flush(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

function unmount(): void {
  const mounted = root
  root = null
  if (mounted !== null) act(() => mounted.unmount())
}

function mount(next: StudioController, nextMenu: FakeMenu): void {
  unmount()
  controller = next
  menu = nextMenu
  root = createRoot(container)
  act(() => root?.render(<App controller={controller} native={null} menu={menu} />))
}

/** A demo controller whose exports count requests; `cancel` makes each export behave as a cancelled save. */
let releasePreview: () => void = () => undefined

/** Demo controller whose exports count requests; `gatePreview` holds previews until `releasePreview`. */
function demoController(
  options: { cancel?: boolean; hang?: boolean; gatePreview?: boolean } = {},
): StudioController {
  const demo = createDemoProcessingAdapter({ urlApi: countingUrlApi() })
  const gate = new Promise<void>((resolve) => {
    releasePreview = resolve
  })
  const adapter: ProcessingAdapter = {
    ...demo,
    preview: (...args) =>
      options.gatePreview === true ? gate.then(() => demo.preview(...args)) : demo.preview(...args),
    export: (...args) => {
      exportRequests += 1
      if (options.cancel === true) return Promise.reject(new ExportCancelledError())
      if (options.hang === true) return new Promise<never>(() => undefined)
      return demo.export(...args)
    },
  }
  return createStudioController({ adapter })
}

/** Menu whose listeners are tracked one by one, so a stale listener can still receive events. */
class PerListenerMenu implements NativeMenu {
  readonly live = new Set<(action: MenuAction) => void>()

  setState(): Promise<void> {
    return Promise.resolve()
  }

  listen(handler: (action: MenuAction) => void): Promise<() => void> {
    this.live.add(handler)
    return new Promise((resolve) =>
      setTimeout(() => resolve(() => void this.live.delete(handler)), 0),
    )
  }

  emitToAll(action: MenuAction): void {
    for (const handler of [...this.live]) handler(action)
  }
}

beforeEach(async () => {
  window.localStorage.setItem('image-to-binary-theme-choice', 'dark')
  container = document.createElement('div')
  document.body.append(container)
  exportRequests = 0
  mount(demoController(), new FakeMenu())
  await controller.loadDemo()
  await flush()
})

afterEach(() => {
  unmount()
  controller.dispose()
  container.remove()
  window.localStorage.removeItem('image-to-binary-theme-choice')
  vi.unstubAllGlobals()
})

function buttonNamed(label: string): HTMLButtonElement {
  const found = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  if (found === undefined) throw new Error(`no button ${label}`)
  return found
}

function exportButton(): HTMLButtonElement {
  const found = container.querySelector<HTMLButtonElement>('.export-section button.button-primary')
  if (found === null) throw new Error('no export control')
  return found
}

function inputNamed(name: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[name="${name}"]`)
  if (found === null) throw new Error(`no input ${name}`)
  return found
}

function typeInto(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setter?.call(input, value)
  act(() => {
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function pressEscape(target: EventTarget): void {
  act(() => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
  })
}

describe('menu state', () => {
  it('reports the gates to the menu, so disabled items match disabled buttons', () => {
    expect(menu.last).toEqual({
      export: true,
      render: true,
      original: true,
      artwork: true,
      cancel: false,
    })
  })

  it('reports nothing enabled before an image is loaded', () => {
    const empty = new FakeMenu()
    mount(createStudioController(), empty)
    expect(empty.last).toEqual({
      export: false,
      render: false,
      original: false,
      artwork: false,
      cancel: false,
    })
  })

  it('reports cancel only while the studio is busy', async () => {
    const busy = new FakeMenu()
    mount(demoController({ hang: true }), busy)
    await controller.loadDemo()
    await flush()
    busy.emit('export')
    await flush()
    expect(busy.last.cancel).toBe(true)
    expect(busy.last.export).toBe(false)
  })
})

describe('menu actions use the button gates', () => {
  it('refuses a menu render and export while the ramp draft is invalid', async () => {
    act(() => container.querySelector<HTMLInputElement>('input[value="ascii"]')?.click())
    typeInto(inputNamed('ramp'), 'café')
    await flush()
    expect(menu.last.export).toBe(false)
    expect(menu.last.render).toBe(false)
    const before = exportRequests
    const preview = vi.spyOn(controller, 'preview')
    menu.emit('export')
    menu.emit('render')
    await flush()
    expect(exportRequests).toBe(before)
    expect(preview).not.toHaveBeenCalled()
  })

  it('does not start a second export while one is running', async () => {
    mount(demoController({ hang: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    menu.emit('export')
    await flush()
    expect(exportRequests).toBe(1)
    menu.emit('export')
    await flush()
    expect(exportRequests).toBe(1)
  })

  it('ignores a menu export when no source is loaded', async () => {
    mount(createStudioController(), new FakeMenu())
    menu.emit('export')
    await flush()
    expect(controller.getSnapshot().export.status).toBe('idle')
  })

  it('runs a menu export through the same request as the button', async () => {
    menu.emit('export')
    await flush()
    expect(exportRequests).toBe(1)
  })
})

describe('Escape', () => {
  it('does nothing while idle', () => {
    const cancel = vi.spyOn(controller, 'cancel')
    pressEscape(document.body)
    expect(cancel).not.toHaveBeenCalled()
  })

  it('cancels a running export from the page', async () => {
    mount(demoController({ hang: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    menu.emit('export')
    await flush()
    expect(controller.getSnapshot().export.status).toBe('running')
    pressEscape(document.body)
    await flush()
    expect(controller.getSnapshot().export.status).toBe('idle')
  })

  it('never cancels from a text field such as the ramp', async () => {
    mount(demoController({ hang: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    act(() => container.querySelector<HTMLInputElement>('input[value="ascii"]')?.click())
    await flush()
    menu.emit('export')
    await flush()
    pressEscape(inputNamed('ramp'))
    await flush()
    expect(controller.getSnapshot().export.status).toBe('running')
  })
})

describe('focus', () => {
  it('returns focus to Open image after Reset', () => {
    act(() => buttonNamed('Reset').click())
    expect(document.activeElement).toBe(buttonNamed('Open image'))
  })

  it('returns focus to Open image after a menu open settles, when focus began on body', async () => {
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    menu.emit('open')
    await flush()
    expect(document.activeElement).toBe(buttonNamed('Open image'))
  })

  it('moves focus off body after a menu action fired with focus on body', () => {
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    menu.emit('original')
    expect(document.activeElement).not.toBe(document.body)
    expect(document.activeElement?.id).toBe('canvas-heading')
  })

  it('lands on Export after a menu export is cancelled, so focus never falls to body', async () => {
    mount(demoController({ cancel: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    menu.emit('export')
    await flush()
    expect(controller.getSnapshot().export.status).toBe('idle')
    expect(document.activeElement).toBe(exportButton())
  })
})

describe('settled export focus', () => {
  it('leaves focus alone when it sits on a control outside the export panel', async () => {
    mount(demoController({ cancel: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    const open = buttonNamed('Open image')
    act(() => open.focus())
    menu.emit('export')
    await flush()
    expect(controller.getSnapshot().export.status).toBe('idle')
    expect(document.activeElement).toBe(open)
  })
})

describe('appearance', () => {
  it('follows a live system change without losing the loaded image', async () => {
    window.localStorage.removeItem('image-to-binary-theme-choice')
    let listener: (() => void) | null = null
    let light = false
    const query = {
      get matches() {
        return light
      },
      addEventListener: (_: string, handler: () => void) => {
        listener = handler
      },
      removeEventListener: () => undefined,
    }
    vi.stubGlobal('matchMedia', () => query)
    mount(demoController(), new FakeMenu())
    await controller.loadDemo()
    await flush()
    expect(document.documentElement.dataset.theme).toBe('dark')
    const before = controller.getSnapshot().metadata
    light = true
    act(() => listener?.())
    expect(document.documentElement.dataset.theme).toBe('light')
    expect(controller.getSnapshot().metadata).toBe(before)
    expect(container.textContent).toContain(before?.name ?? '')
  })

  it('keeps the chosen theme in the store under its own key', () => {
    act(() => themeRadio('light').click())
    expect(document.documentElement.dataset.theme).toBe('light')
    expect(window.localStorage.getItem('image-to-binary-theme-choice')).toBe('light')
  })
})

function themeRadio(value: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`.theme-choice input[value="${value}"]`)
  if (found === null) throw new Error(`no theme radio ${value}`)
  return found
}

describe('theme choice', () => {
  it('offers System, Light and Dark and marks the stored choice', () => {
    const values = [...container.querySelectorAll<HTMLInputElement>('.theme-choice input')].map(
      (input) => input.value,
    )
    expect(values).toEqual(['system', 'light', 'dark'])
    expect(themeRadio('dark').checked).toBe(true)
  })

  it('stores System explicitly and returns to the operating appearance', () => {
    act(() => themeRadio('light').click())
    act(() => themeRadio('system').click())
    expect(window.localStorage.getItem('image-to-binary-theme-choice')).toBe('system')
    expect(themeRadio('system').checked).toBe(true)
  })

  it('System follows a live operating-appearance change', () => {
    let listener: (() => void) | null = null
    let light = true
    const query = {
      get matches() {
        return light
      },
      addEventListener: (_: string, handler: () => void) => {
        listener = handler
      },
      removeEventListener: () => undefined,
    }
    vi.stubGlobal('matchMedia', () => query)
    window.localStorage.removeItem('image-to-binary-theme-choice')
    mount(demoController(), new FakeMenu())
    expect(document.documentElement.dataset.theme).toBe('light')
    light = false
    act(() => listener?.())
    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('a stored explicit choice survives a remount', () => {
    act(() => themeRadio('light').click())
    mount(demoController(), new FakeMenu())
    expect(themeRadio('light').checked).toBe(true)
    expect(document.documentElement.dataset.theme).toBe('light')
  })
})

describe('focus after a busy phase', () => {
  async function settle(): Promise<void> {
    await act(async () => {
      releasePreview()
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    await flush()
  }

  it('returns focus to the control that held it when the busy state began', async () => {
    mount(demoController({ gatePreview: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    const original = buttonNamed('Original')
    act(() => original.focus())
    menu.emit('render')
    await flush()
    expect(controller.getSnapshot().status).toBe('processing')
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    expect(document.activeElement).toBe(document.body)
    await settle()
    expect(controller.getSnapshot().status).toBe('ready')
    expect(document.activeElement).toBe(original)
  })

  it('does not steal focus from a control the user moved to during the busy state', async () => {
    mount(demoController({ gatePreview: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    act(() => buttonNamed('Original').focus())
    menu.emit('render')
    await flush()
    const reset = buttonNamed('Reset')
    act(() => reset.focus())
    await settle()
    expect(controller.getSnapshot().status).toBe('ready')
    expect(document.activeElement).toBe(reset)
  })
})

describe('gates read at the moment of action', () => {
  it('refuses a shortcut that arrives before React commits the running export', async () => {
    mount(demoController({ hang: true }), new FakeMenu())
    await controller.loadDemo()
    await flush()
    act(() => {
      void controller.startExport('png')
      menu.emit('export')
    })
    await flush()
    expect(exportRequests).toBe(1)
  })
})

describe('StrictMode listener', () => {
  it('runs one action per event even while a stale listener is still registered', async () => {
    const strictMenu = new PerListenerMenu()
    mount(demoController(), new FakeMenu())
    await controller.loadDemo()
    await flush()
    unmount()
    const preview = vi.spyOn(controller, 'preview')
    root = createRoot(container)
    act(() =>
      root?.render(
        <StrictMode>
          <App controller={controller} native={null} menu={strictMenu} />
        </StrictMode>,
      ),
    )
    act(() => strictMenu.emitToAll('artwork'))
    await flush()
    expect(preview).toHaveBeenCalledTimes(1)
  })
})

describe('slider labels', () => {
  it('keeps each output outside its label, linked by htmlFor', () => {
    for (const id of ['threshold', 'cell-size', 'font-size']) {
      const label = container.querySelector<HTMLLabelElement>(`label[for="${id}"]`)
      expect(label?.querySelector('output')).toBeNull()
      expect(container.querySelector(`output[for="${id}"]`)).not.toBeNull()
    }
  })
})
