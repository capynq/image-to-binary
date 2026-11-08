/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { countingUrlApi } from './export/exportTestSupport'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import type { PreviewKind, ProcessingAdapter } from './processing/types'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'
import { AUTO_RENDER_DELAY_MS } from './useAutoRender'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

interface PreviewCall {
  readonly kind: PreviewKind
  readonly quiet: boolean
}

let container: HTMLDivElement
let root: Root | null = null
let controller: StudioController
let previews: PreviewCall[]
let exportCount: number
let holdQuiet: Promise<void> | null
let releaseQuiet: () => void
let holdExport: Promise<void> | null
let releaseExport: () => void
let failQuiet: boolean

function gate(): { readonly promise: Promise<void>; readonly open: () => void } {
  let open: () => void = () => undefined
  const promise = new Promise<void>((resolve) => {
    open = resolve
  })
  return { promise, open }
}

/**
 * A demo controller that records every preview and export. Quiet previews can be held or made to fail,
 * exports can be held, and the original preview is served as artwork because the demo adapter has none.
 */
function buildController(): StudioController {
  const demo = createDemoProcessingAdapter({ urlApi: countingUrlApi() })
  const adapter: ProcessingAdapter = {
    ...demo,
    preview: async (session, source, settings, context, kind, onProgress) => {
      const quiet = onProgress === undefined
      previews.push({ kind: kind ?? 'artwork', quiet })
      if (quiet && holdQuiet !== null) await holdQuiet
      if (quiet && failQuiet) throw new Error('The quiet render failed.')
      return demo.preview(
        session,
        source,
        settings,
        context,
        kind === 'original' ? 'artwork' : kind,
        onProgress,
      )
    },
    export: async (session, source, format, settings, context, onProgress) => {
      exportCount += 1
      if (holdExport !== null) await holdExport
      return demo.export(session, source, format, settings, context, onProgress)
    },
  }
  return createStudioController({ adapter })
}

function mount(): void {
  unmount()
  root = createRoot(container)
  act(() => root?.render(<App controller={controller} native={null} />))
}

function unmount(): void {
  const mounted = root
  root = null
  if (mounted !== null) act(() => mounted.unmount())
}

async function advance(ms: number): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms)
  })
}

function setRaw(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setter?.call(input, value)
  act(() => {
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function field(name: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[name="${name}"]`)
  if (found === null) throw new Error(`no field ${name}`)
  return found
}

function button(label: string): HTMLButtonElement {
  const found = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  if (found === undefined) throw new Error(`no button ${label}`)
  return found
}

function radio(value: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`)
  if (found === null) throw new Error(`no radio for ${value}`)
  return found
}

function rampInput(): HTMLInputElement {
  return field('ramp')
}

function changeCellSize(value: number): void {
  setRaw(field('cellSize'), String(value))
}

function pressEscape(): void {
  act(() => {
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
  })
}

function holdQuietRenders(): void {
  const hold = gate()
  holdQuiet = hold.promise
  releaseQuiet = hold.open
}

function holdExports(): void {
  const hold = gate()
  holdExport = hold.promise
  releaseExport = hold.open
}

function quietPreviews(): PreviewCall[] {
  return previews.filter((call) => call.quiet)
}

beforeEach(async () => {
  vi.useFakeTimers()
  window.localStorage.setItem('image-to-binary-theme', 'dark')
  container = document.createElement('div')
  document.body.append(container)
  previews = []
  exportCount = 0
  holdQuiet = null
  holdExport = null
  failQuiet = false
  releaseQuiet = () => undefined
  releaseExport = () => undefined
})

afterEach(() => {
  unmount()
  controller.dispose()
  container.remove()
  window.localStorage.removeItem('image-to-binary-theme')
  vi.useRealTimers()
})

describe('automatic render after edits', () => {
  beforeEach(async () => {
    controller = buildController()
    await controller.loadDemo()
    await controller.preview()
    previews = []
    mount()
    act(() => radio('artwork').click())
    previews = []
  })

  it('coalesces five rapid slider changes into exactly one quiet render, after the delay', async () => {
    for (const value of [8, 9, 10, 11]) {
      changeCellSize(value)
      await advance(50)
    }
    changeCellSize(12)
    expect(previews).toEqual([])
    await advance(AUTO_RENDER_DELAY_MS - 1)
    expect(previews).toEqual([])
    await advance(1)
    expect(previews).toEqual([{ kind: 'artwork', quiet: true }])
    await advance(2 * AUTO_RENDER_DELAY_MS)
    expect(previews).toHaveLength(1)
    expect(controller.getSnapshot().previewRevision).toBe(controller.getSnapshot().settingsRevision)
  })

  it('keeps the artwork on screen, status ready and no progress row during a quiet render', async () => {
    const shown = controller.getSnapshot().preview
    changeCellSize(9)
    await advance(AUTO_RENDER_DELAY_MS)
    expect(controller.getSnapshot().status).toBe('ready')
    expect(container.querySelector('.busy-row')).toBeNull()
    expect(controller.getSnapshot().preview).not.toBe(shown)
  })

  it('shows an accessible Updating badge and dims the stale artwork until the render lands', async () => {
    changeCellSize(9)
    const region = container.querySelector('[role="status"][aria-live="polite"]')
    expect(region?.textContent).toBe('Updating…')
    expect(container.querySelector('.artifact-frame')?.getAttribute('aria-busy')).toBe('true')
    expect(container.querySelector('.artifact-frame')?.classList.contains('is-refreshing')).toBe(
      true,
    )
    await advance(AUTO_RENDER_DELAY_MS + 1)
    expect(region?.textContent).toBe('')
    expect(container.querySelector('.artifact-frame')?.hasAttribute('aria-busy')).toBe(false)
  })

  it('keeps Render and Export enabled while the quiet render runs', async () => {
    holdQuietRenders()
    changeCellSize(9)
    await advance(AUTO_RENDER_DELAY_MS)
    expect(controller.getSnapshot().artworkUpdating).toBe(true)
    expect(button('Render artwork').disabled).toBe(false)
    expect(button('Export PNG').disabled).toBe(false)
    releaseQuiet()
  })

  it('a manual Render flushes at once, cancels the pending render and runs non-quietly', async () => {
    changeCellSize(9)
    await advance(100)
    act(() => button('Render artwork').click())
    await act(async () => {
      await Promise.resolve()
    })
    expect(previews).toEqual([{ kind: 'artwork', quiet: false }])
    await advance(AUTO_RENDER_DELAY_MS * 3)
    expect(previews).toHaveLength(1)
    expect(controller.getSnapshot().previewRevision).toBe(controller.getSnapshot().settingsRevision)
  })

  it('Escape cancels an in-flight quiet render and does not re-fire until the next change', async () => {
    holdQuietRenders()
    changeCellSize(9)
    await advance(AUTO_RENDER_DELAY_MS)
    expect(quietPreviews()).toHaveLength(1)
    expect(controller.getSnapshot().artworkUpdating).toBe(true)
    pressEscape()
    expect(controller.getSnapshot().artworkUpdating).toBe(false)
    expect(controller.getSnapshot().status).toBe('cancelled')
    releaseQuiet()
    await advance(AUTO_RENDER_DELAY_MS * 4)
    expect(quietPreviews()).toHaveLength(1)
    expect(controller.getSnapshot().preview).not.toBeNull()
    expect(controller.getSnapshot().previewRevision).not.toBe(
      controller.getSnapshot().settingsRevision,
    )
    holdQuiet = null
    changeCellSize(10)
    await advance(AUTO_RENDER_DELAY_MS)
    expect(quietPreviews()).toHaveLength(2)
    expect(controller.getSnapshot().previewRevision).toBe(controller.getSnapshot().settingsRevision)
  })

  it('Escape drops a pending render that has not started yet', async () => {
    changeCellSize(9)
    await advance(100)
    pressEscape()
    await advance(AUTO_RENDER_DELAY_MS * 3)
    expect(previews).toEqual([])
    expect(container.querySelector('.updating-badge')).toBeNull()
  })

  it('a failed quiet render is not retried for the same revision', async () => {
    failQuiet = true
    changeCellSize(9)
    await advance(AUTO_RENDER_DELAY_MS)
    expect(quietPreviews()).toHaveLength(1)
    expect(controller.getSnapshot().status).toBe('error')
    await advance(AUTO_RENDER_DELAY_MS * 4)
    expect(quietPreviews()).toHaveLength(1)
    failQuiet = false
    changeCellSize(10)
    await advance(AUTO_RENDER_DELAY_MS)
    expect(quietPreviews()).toHaveLength(2)
    expect(controller.getSnapshot().status).toBe('ready')
  })

  it('an invalid ramp draft blocks the automatic render until the draft is fixed', async () => {
    act(() => radio('ascii').click())
    await advance(AUTO_RENDER_DELAY_MS)
    expect(quietPreviews()).toHaveLength(1)
    setRaw(rampInput(), '')
    changeCellSize(9)
    await advance(AUTO_RENDER_DELAY_MS * 3)
    expect(quietPreviews()).toHaveLength(1)
    expect(controller.getSnapshot().previewRevision).not.toBe(
      controller.getSnapshot().settingsRevision,
    )
    setRaw(rampInput(), '#')
    await advance(AUTO_RENDER_DELAY_MS)
    expect(quietPreviews()).toHaveLength(2)
  })

  it('starting an export clears the pending render and re-arms once a stale export ends', async () => {
    holdExports()
    changeCellSize(9)
    await advance(100)
    act(() => radio('text').click())
    act(() => button('Export text').click())
    await advance(AUTO_RENDER_DELAY_MS * 4)
    expect(exportCount).toBe(1)
    expect(previews).toEqual([])
    releaseExport()
    holdExport = null
    await advance(AUTO_RENDER_DELAY_MS)
    expect(quietPreviews()).toHaveLength(1)
    expect(controller.getSnapshot().previewRevision).toBe(controller.getSnapshot().settingsRevision)
  })
})

describe('automatic render after import', () => {
  beforeEach(() => {
    controller = buildController()
    mount()
  })

  it('renders the artwork quietly once the original preview settles, without the delay', async () => {
    act(() => button('Load demo').click())
    await advance(0)
    expect(previews).toEqual([
      { kind: 'original', quiet: false },
      { kind: 'artwork', quiet: true },
    ])
    expect(controller.getSnapshot().preview).not.toBeNull()
    expect(controller.getSnapshot().previewRevision).toBe(controller.getSnapshot().settingsRevision)
    expect(controller.getSnapshot().artworkUpdating).toBe(false)
    expect(radio('original').checked).toBe(true)
    expect(container.querySelector('.busy-row')).toBeNull()
  })
})
