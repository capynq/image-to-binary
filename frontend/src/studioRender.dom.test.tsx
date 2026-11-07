/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import App from './App'
import { countingUrlApi } from './export/exportTestSupport'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import { DEFAULT_RAMP } from './processing/protocol'
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
