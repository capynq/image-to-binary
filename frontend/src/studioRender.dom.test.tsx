/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { exportFilename } from './export/downloadName'
import { countingUrlApi } from './export/exportTestSupport'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import { DEFAULT_BACKGROUND, DEFAULT_RAMP } from './processing/protocol'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'
import type { ProcessingAdapter } from './processing/types'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

let container: HTMLDivElement
let root: Root | null = null
let controller: StudioController
let exportRequests: Array<Parameters<ProcessingAdapter['export']>>

beforeEach(async () => {
  window.localStorage.setItem('image-to-binary-theme', 'dark')
  container = document.createElement('div')
  document.body.append(container)
  const demo = createDemoProcessingAdapter({ urlApi: countingUrlApi() })
  exportRequests = []
  controller = createStudioController({
    adapter: {
      ...demo,
      export: (...args) => {
        exportRequests.push(args)
        return demo.export(...args)
      },
    },
  })
  await controller.loadDemo()
  root = createRoot(container)
  act(() => root?.render(<App controller={controller} native={null} />))
})

afterEach(() => {
  if (root !== null) {
    const mounted = root
    root = null
    act(() => mounted.unmount())
  }
  controller.dispose()
  container.remove()
  window.localStorage.removeItem('image-to-binary-theme')
})

function radio(value: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`)
  if (found === null) throw new Error(`no radio for ${value}`)
  return found
}

function rampInput(): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>('input[name="ramp"]')
  if (found === null) throw new Error('no ramp field')
  return found
}

function button(label: string): HTMLButtonElement {
  const found = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  if (found === undefined) throw new Error(`no button ${label}`)
  return found
}

function typeInto(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setter?.call(input, value)
  act(() => {
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('studio render modes', () => {
  it('starts in classic binary with no ramp field and the threshold available', () => {
    expect(controller.getSnapshot().settings).toMatchObject({ mode: 'binary', ramp: DEFAULT_RAMP })
    expect(radio('binary').checked).toBe(true)
    expect(container.querySelector('input[name="ramp"]')).toBeNull()
    expect(container.querySelector<HTMLInputElement>('input#threshold')?.disabled).toBe(false)
  })

  it('switches to ASCII through the controller and disables the threshold it no longer uses', () => {
    act(() => radio('ascii').click())
    expect(controller.getSnapshot().settings).toMatchObject({ mode: 'ascii', ramp: DEFAULT_RAMP })
    expect(rampInput().value).toBe(DEFAULT_RAMP)
    expect(container.querySelector<HTMLInputElement>('input#threshold')?.disabled).toBe(true)
  })

  it('applies a valid custom ramp to the settings', () => {
    act(() => radio('ascii').click())
    typeInto(rampInput(), ' .#@')
    expect(controller.getSnapshot().settings.ramp).toBe(' .#@')
    expect(rampInput().getAttribute('aria-invalid')).toBe('false')
  })

  it('keeps the accepted source, the last good preview and the last good ramp on an invalid ramp', async () => {
    act(() => radio('ascii').click())
    await controller.preview()
    const before = controller.getSnapshot()
    expect(before.preview).not.toBeNull()
    const generation = controller.getSnapshot().generation
    typeInto(rampInput(), 'café')
    const after = controller.getSnapshot()
    expect(after.metadata).toEqual(before.metadata)
    expect(after.preview).toBe(before.preview)
    expect(after.status).toBe('ready')
    expect(after.error).toBeNull()
    expect(after.generation).toBe(generation)
    expect(after.settings.ramp).toBe(DEFAULT_RAMP)
    expect(container.querySelector('.field-error')?.textContent).toBe(
      'Not applied: ramp must contain only printable ASCII characters.',
    )
    expect(rampInput().getAttribute('aria-invalid')).toBe('true')
    expect(container.querySelector('.error-panel')).toBeNull()
  })

  it('blocks a new render and PNG export while the ramp is invalid', () => {
    act(() => radio('ascii').click())
    typeInto(rampInput(), '')
    expect(button('Render artwork').disabled).toBe(true)
    expect(button('Export PNG').disabled).toBe(true)
    expect(container.textContent).toContain('Fix the ramp to enable PNG export.')
    typeInto(rampInput(), '#')
    expect(button('Render artwork').disabled).toBe(false)
  })

  it('keeps the chosen ramp when the mode moves away from ASCII and back', () => {
    act(() => radio('ascii').click())
    typeInto(rampInput(), '.oO@')
    act(() => radio('block').click())
    expect(controller.getSnapshot().settings).toMatchObject({ mode: 'block', ramp: '.oO@' })
    act(() => radio('ascii').click())
    expect(rampInput().value).toBe('.oO@')
  })
})

describe('studio render gates and requests', () => {
  it('disables the Artwork tab with a stated reason while the ramp is invalid', () => {
    act(() => radio('ascii').click())
    typeInto(rampInput(), '')
    const tab = button('Artwork')
    expect(tab.disabled).toBe(true)
    const reason = document.getElementById(tab.getAttribute('aria-describedby') ?? '')
    expect(reason?.textContent).toBe('Fix the ramp to show the artwork.')
    act(() => tab.click())
    expect(controller.getSnapshot().status).toBe('ready')
    expect(button('Original').getAttribute('aria-pressed')).toBe('true')
    typeInto(rampInput(), '#')
    expect(button('Artwork').disabled).toBe(false)
    expect(button('Artwork').hasAttribute('aria-describedby')).toBe(false)
  })

  it('resyncs the ramp field when the applied ramp changes from outside', () => {
    act(() => radio('ascii').click())
    typeInto(rampInput(), '')
    act(() => {
      controller.updateSettings({ ...controller.getSnapshot().settings, ramp: '.#%' })
    })
    expect(rampInput().value).toBe('.#%')
    expect(rampInput().getAttribute('aria-invalid')).toBe('false')
  })

  it('the exported request carries the edited valid ramp', async () => {
    act(() => radio('ascii').click())
    typeInto(rampInput(), ' .#@')
    await controller.startExport().catch(() => undefined)
    const requested = exportRequests.at(-1)?.[3]
    expect(requested).toMatchObject({ mode: 'ascii', ramp: ' .#@' })
  })
})

function field(name: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[name="${name}"]`)
  if (found === null) throw new Error(`no field ${name}`)
  return found
}

function setRaw(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setter?.call(input, value)
  act(() => {
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('studio render colour gates and style requests', () => {
  it('blocks render, PNG export and the Artwork tab while a colour draft is invalid', async () => {
    await controller.preview()
    const before = controller.getSnapshot()
    const update = vi.spyOn(controller, 'updateSettings')
    setRaw(field('background'), '#12')
    expect(button('Render artwork').disabled).toBe(true)
    expect(button('Export PNG').disabled).toBe(true)
    expect(container.textContent).toContain('Fix the colour to enable PNG export.')
    const tab = button('Artwork')
    expect(tab.disabled).toBe(true)
    const reason = document.getElementById(tab.getAttribute('aria-describedby') ?? '')
    expect(reason?.textContent).toBe('Fix the colour to show the artwork.')
    expect(update).not.toHaveBeenCalled()
    const after = controller.getSnapshot()
    expect(after.settings.background).toBe(DEFAULT_BACKGROUND)
    expect(after.preview).toBe(before.preview)
    expect(after.generation).toBe(before.generation)
    expect(after.status).toBe('ready')
    expect(field('background').getAttribute('aria-invalid')).toBe('true')
    expect(container.querySelector('.error-panel')).toBeNull()
  })

  it('recovers from an invalid colour draft by picking the accepted colour', () => {
    setRaw(field('background'), '#12')
    expect(button('Render artwork').disabled).toBe(true)
    const picker = field('backgroundPicker')
    act(() => {
      picker.click()
    })
    expect(field('background').value).toBe(DEFAULT_BACKGROUND)
    expect(field('background').getAttribute('aria-invalid')).toBe('false')
    expect(button('Render artwork').disabled).toBe(false)
    expect(button('Export PNG').disabled).toBe(false)
    expect(button('Artwork').disabled).toBe(false)
  })

  it('applies a valid typed colour and lifts the gate', () => {
    setRaw(field('foreground'), '#ffffff')
    expect(controller.getSnapshot().settings.foreground).toBe('#ffffff')
    expect(button('Render artwork').disabled).toBe(false)
    expect(button('Export PNG').disabled).toBe(false)
    expect(button('Artwork').disabled).toBe(false)
  })

  it('applies a picker colour immediately and keeps the hex field in step', () => {
    setRaw(field('backgroundPicker'), '#202020')
    expect(controller.getSnapshot().settings.background).toBe('#202020')
    expect(field('background').value).toBe('#202020')
  })

  it('names the ramp first when the ramp and a colour are both invalid', () => {
    act(() => radio('ascii').click())
    typeInto(rampInput(), '')
    setRaw(field('foreground'), 'green')
    expect(container.textContent).toContain('Fix the ramp to enable PNG export.')
    expect(container.textContent).not.toContain('Fix the colour to enable PNG export.')
  })

  it('updates colour mode, contrast and invert through the controller', () => {
    act(() => {
      const source = container.querySelector<HTMLInputElement>('input[value="source"]')
      source?.click()
    })
    expect(controller.getSnapshot().settings.colorMode).toBe('source')
    setRaw(field('contrast'), '-30')
    expect(controller.getSnapshot().settings.contrast).toBe(-30)
    act(() => field('invert').click())
    expect(controller.getSnapshot().settings.invert).toBe(true)
  })

  it('sends the edited colour style with the export request', async () => {
    setRaw(field('foreground'), '#112233')
    await controller.startExport().catch(() => undefined)
    const requested = exportRequests.at(-1)?.[3]
    expect(requested).toMatchObject({ foreground: '#112233', background: DEFAULT_BACKGROUND })
  })
})

describe('studio text export', () => {
  const originalCreate = URL.createObjectURL
  const originalRevoke = URL.revokeObjectURL

  beforeEach(() => {
    Object.assign(URL, {
      createObjectURL: () => 'blob:studio-text',
      revokeObjectURL: () => undefined,
    })
  })

  afterEach(() => {
    Object.assign(URL, { createObjectURL: originalCreate, revokeObjectURL: originalRevoke })
  })

  function anchor(label: string): HTMLAnchorElement {
    const found = [...container.querySelectorAll<HTMLAnchorElement>('a')].find(
      (candidate) => candidate.textContent?.trim() === label,
    )
    if (found === undefined) throw new Error(`no link ${label}`)
    return found
  }

  async function settleExport(): Promise<void> {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
  }

  it('exports text through the controller with its format and offers a .txt download', async () => {
    act(() => radio('text').click())
    expect(button('Export text').disabled).toBe(false)
    act(() => button('Export text').click())
    await settleExport()
    expect(exportRequests.at(-1)?.[2]).toBe('text')
    const name = controller.getSnapshot().metadata?.name ?? ''
    expect(anchor('Download text').getAttribute('download')).toBe(exportFilename(name, 'text'))
    expect(container.querySelector('.export-status')?.textContent).toMatch(/^Text export ready, /)
  })

  it('requests PNG again once the format is switched back, and never as text', async () => {
    act(() => radio('text').click())
    act(() => radio('png').click())
    act(() => button('Export PNG').click())
    await settleExport()
    expect(exportRequests.at(-1)?.[2]).toBe('png')
    expect(container.querySelector('.export-status')?.textContent).toMatch(/^PNG export failed/)
  })

  it('blocks text export and names it while the ramp is invalid, as PNG export is', () => {
    act(() => radio('text').click())
    act(() => radio('ascii').click())
    typeInto(rampInput(), '')
    expect(button('Export text').disabled).toBe(true)
    expect(container.textContent).toContain('Fix the ramp to enable text export.')
    typeInto(rampInput(), '#')
    expect(button('Export text').disabled).toBe(false)
  })
})

describe('studio download name follows the artifact format', () => {
  const originalCreate = URL.createObjectURL
  const originalRevoke = URL.revokeObjectURL
  const PNG_BYTES = Uint8Array.of(137, 80, 78, 71, 13, 10, 26, 10)
  let blobs: Blob[] = []

  beforeEach(async () => {
    blobs = []
    Object.assign(URL, {
      createObjectURL: (object: Blob) => {
        blobs.push(object)
        return 'blob:studio-format'
      },
      revokeObjectURL: () => undefined,
    })
    controller.dispose()
    const demo = createDemoProcessingAdapter({ urlApi: countingUrlApi() })
    controller = createStudioController({
      adapter: {
        ...demo,
        export: async (...args) => {
          if (args[2] !== 'png') return demo.export(...args)
          return {
            kind: 'blob',
            format: 'png',
            blob: new Blob([PNG_BYTES], { type: 'image/png' }),
            byteLength: PNG_BYTES.byteLength,
            dispose: () => undefined,
          }
        },
      },
    })
    await controller.loadDemo()
    act(() => root?.render(<App controller={controller} native={null} />))
  })

  afterEach(() => {
    Object.assign(URL, { createObjectURL: originalCreate, revokeObjectURL: originalRevoke })
  })

  function anchor(label: string): HTMLAnchorElement {
    const found = [...container.querySelectorAll<HTMLAnchorElement>('a')].find(
      (candidate) => candidate.textContent?.trim() === label,
    )
    if (found === undefined) throw new Error(`no link ${label}`)
    return found
  }

  async function settleExport(): Promise<void> {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
  }

  it('keeps the PNG name and type after the format radio moves to text', async () => {
    act(() => button('Export PNG').click())
    await settleExport()
    const name = exportFilename(controller.getSnapshot().metadata?.name ?? '', 'png')
    expect(name).toMatch(/-binary\.png$/)
    expect(anchor('Download PNG').getAttribute('download')).toBe(name)
    act(() => radio('text').click())
    expect(anchor('Download PNG').getAttribute('download')).toBe(name)
    expect(blobs.at(-1)?.type).toBe('image/png')
  })

  it('keeps the text name and type after the format radio moves back to PNG', async () => {
    act(() => radio('text').click())
    act(() => button('Export text').click())
    await settleExport()
    const name = exportFilename(controller.getSnapshot().metadata?.name ?? '', 'text')
    expect(name).toMatch(/-binary\.txt$/)
    expect(anchor('Download text').getAttribute('download')).toBe(name)
    act(() => radio('png').click())
    expect(anchor('Download text').getAttribute('download')).toBe(name)
    expect(blobs.at(-1)?.type).toBe('text/plain;charset=utf-8')
  })
})
