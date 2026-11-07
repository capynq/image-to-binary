/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RenderStyleFields, colourProblem } from './RenderStyleFields'
import type { RenderStyleFieldsProps } from './RenderStyleFields'
import { MAX_CONTRAST, MIN_CONTRAST } from './processing/bounds'
import { DEFAULT_BACKGROUND, DEFAULT_FOREGROUND, FOREGROUND_MESSAGE } from './processing/protocol'
import { DEFAULT_SETTINGS } from './processing/types'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) {
    const mounted = root
    root = null
    act(() => mounted.unmount())
  }
  container.remove()
})

function show(overrides: Partial<RenderStyleFieldsProps> = {}): RenderStyleFieldsProps {
  const props: RenderStyleFieldsProps = {
    settings: DEFAULT_SETTINGS,
    foregroundDraft: DEFAULT_FOREGROUND,
    backgroundDraft: DEFAULT_BACKGROUND,
    onColourChange: vi.fn(),
    onColourDraftChange: vi.fn(),
    onColorModeChange: vi.fn(),
    onContrastChange: vi.fn(),
    onInvertChange: vi.fn(),
    ...overrides,
  }
  const target = root ?? createRoot(container)
  act(() => target.render(<RenderStyleFields {...props} />))
  return props
}

function input(selector: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(selector)
  if (found === null) throw new Error(`no input for ${selector}`)
  return found
}

function setValue(element: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setter?.call(element, value)
  act(() => {
    element.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function errorOf(name: string): HTMLElement {
  const field = input(`input[name="${name}"]`).closest('.colour-field')
  const error = field?.querySelector<HTMLElement>('.field-error')
  if (!error) throw new Error(`no error for ${name}`)
  return error
}

describe('render style fields: colour', () => {
  it('labels the picker and the hex field visibly for each role', () => {
    show()
    const labels = [...container.querySelectorAll('label')].map((label) => label.textContent)
    expect(labels).toContain('Foreground colour')
    expect(labels).toContain('Foreground colour hex')
    expect(labels).toContain('Background colour')
    expect(labels).toContain('Background colour hex')
    expect(input('input[name="foregroundPicker"]').type).toBe('color')
    expect(input('input[name="backgroundPicker"]').type).toBe('color')
    expect(input('input[name="foreground"]').type).toBe('text')
    expect(input('input[name="background"]').type).toBe('text')
  })

  it('shows the accepted colours in the picker and the draft in the hex field', () => {
    show({ settings: { ...DEFAULT_SETTINGS, foreground: '#112233' }, foregroundDraft: '#112233' })
    expect(input('input[name="foregroundPicker"]').value).toBe('#112233')
    expect(input('input[name="foreground"]').value).toBe('#112233')
    expect(input('input[name="background"]').value).toBe(DEFAULT_BACKGROUND)
  })

  it('forwards a valid picker colour immediately', () => {
    const props = show()
    setValue(input('input[name="foregroundPicker"]'), '#ff8800')
    expect(props.onColourChange).toHaveBeenLastCalledWith('foreground', '#ff8800')
    setValue(input('input[name="backgroundPicker"]'), '#101010')
    expect(props.onColourChange).toHaveBeenLastCalledWith('background', '#101010')
  })

  it('reports every keystroke as a draft and forwards only a valid hex colour', () => {
    const props = show()
    setValue(input('input[name="foreground"]'), '#ff88')
    expect(props.onColourDraftChange).toHaveBeenLastCalledWith('foreground', '#ff88')
    expect(props.onColourChange).not.toHaveBeenCalled()
    setValue(input('input[name="foreground"]'), '#ff8800')
    expect(props.onColourDraftChange).toHaveBeenLastCalledWith('foreground', '#ff8800')
    expect(props.onColourChange).toHaveBeenLastCalledWith('foreground', '#ff8800')
  })

  it('links an invalid hex draft to a polite message and does not apply it', () => {
    const props = show({ foregroundDraft: 'green' })
    const field = input('input[name="foreground"]')
    const error = errorOf('foreground')
    expect(field.getAttribute('aria-invalid')).toBe('true')
    expect(error.textContent).toBe(`Not applied: ${FOREGROUND_MESSAGE}.`)
    expect(error.getAttribute('aria-live')).toBe('polite')
    const described = field.getAttribute('aria-describedby')?.split(' ') ?? []
    expect(described).toContain(error.id)
    expect(described.length).toBe(2)
    expect(props.onColourChange).not.toHaveBeenCalled()
  })

  it('names the background role in its own message', () => {
    show({ backgroundDraft: '#12345' })
    expect(errorOf('background').textContent).toBe(
      'Not applied: background must be a #RRGGBB colour.',
    )
    expect(input('input[name="background"]').getAttribute('aria-invalid')).toBe('true')
    expect(input('input[name="foreground"]').getAttribute('aria-invalid')).toBe('false')
  })

  it('reports the colour problem for the App gate', () => {
    expect(colourProblem('#00ff00', 'foreground')).toBeNull()
    expect(colourProblem('#00FF00', 'background')).toBeNull()
    expect(colourProblem('#0f0', 'foreground')).toBe(FOREGROUND_MESSAGE)
  })

  it('keeps every colour control a native element that receives focus', () => {
    show()
    for (const selector of [
      'input[name="foregroundPicker"]',
      'input[name="foreground"]',
      'input[name="backgroundPicker"]',
      'input[name="background"]',
    ]) {
      const control = input(selector)
      expect(control.disabled).toBe(false)
      act(() => control.focus())
      expect(document.activeElement).toBe(control)
    }
  })
})

describe('render style fields: colour mode', () => {
  it('offers monochrome and source colour in a labelled native radio group', () => {
    show()
    expect(container.querySelector('legend')?.textContent).toBe('Colour mode')
    const monochrome = input('input[type="radio"][value="monochrome"]')
    const source = input('input[type="radio"][value="source"]')
    expect(monochrome.checked).toBe(true)
    expect(source.checked).toBe(false)
    expect(monochrome.name).toBe(source.name)
    expect(monochrome.name).not.toBe('')
    expect(
      [...container.querySelectorAll('.mode-option')].map((option) => option.textContent),
    ).toEqual(['Monochrome', 'Source colour'])
  })

  it('reports the chosen colour mode', () => {
    const props = show()
    act(() => input('input[type="radio"][value="source"]').click())
    expect(props.onColorModeChange).toHaveBeenLastCalledWith('source')
  })
})

describe('render style fields: contrast and invert', () => {
  it('bounds the contrast range by the shared limits and shows the value', () => {
    show({ settings: { ...DEFAULT_SETTINGS, contrast: 25 } })
    const range = input('input[name="contrast"]')
    expect(range.type).toBe('range')
    expect(range.min).toBe(String(MIN_CONTRAST))
    expect(range.max).toBe(String(MAX_CONTRAST))
    expect(range.step).toBe('1')
    expect(range.value).toBe('25')
    expect(container.querySelector('output')?.textContent).toBe('25')
    expect(container.textContent).toContain('Contrast')
  })

  it('reports each contrast change as a number', () => {
    const props = show()
    setValue(input('input[name="contrast"]'), '-40')
    expect(props.onContrastChange).toHaveBeenLastCalledWith(-40)
  })

  it('labels the invert checkbox and reports its state', () => {
    const props = show()
    const invert = input('input[name="invert"]')
    expect(invert.type).toBe('checkbox')
    expect(invert.checked).toBe(false)
    expect(container.textContent).toContain('Invert tones')
    act(() => invert.click())
    expect(props.onInvertChange).toHaveBeenLastCalledWith(true)
  })
})

describe('render style fields: defaults', () => {
  it('renders the default settings with no style change', () => {
    show()
    expect(input('input[name="foregroundPicker"]').value).toBe(DEFAULT_FOREGROUND)
    expect(input('input[name="backgroundPicker"]').value).toBe(DEFAULT_BACKGROUND)
    expect(input('input[name="foreground"]').value).toBe(DEFAULT_FOREGROUND)
    expect(input('input[name="background"]').value).toBe(DEFAULT_BACKGROUND)
    expect(input('input[type="radio"][value="monochrome"]').checked).toBe(true)
    expect(input('input[name="contrast"]').value).toBe('0')
    expect(input('input[name="invert"]').checked).toBe(false)
    expect(container.querySelectorAll('.field-error')).toHaveLength(2)
    for (const error of container.querySelectorAll('.field-error')) {
      expect(error.textContent).toBe('')
    }
  })
})

describe('render style fields: stable names and recovery', () => {
  it('keeps the contrast label name fixed while the value changes', () => {
    show({ settings: { ...DEFAULT_SETTINGS, contrast: 0 } })
    const range = input('input[name="contrast"]')
    const labelName = () => container.querySelector(`label[for="${range.id}"]`)?.textContent?.trim()
    expect(labelName()).toBe('Contrast')
    expect(range.getAttribute('aria-valuetext')).toBe('0')
    show({ settings: { ...DEFAULT_SETTINGS, contrast: 37 } })
    expect(labelName()).toBe('Contrast')
    expect(input('input[name="contrast"]').getAttribute('aria-valuetext')).toBe('37')
    expect(container.querySelector(`output[for="${range.id}"]`)?.textContent).toBe('37')
  })

  it('resyncs an invalid hex draft to the accepted colour when the picker is used', () => {
    const onColourDraftChange = vi.fn()
    show({ foregroundDraft: '#zz0000', onColourDraftChange })
    input('input[name="foregroundPicker"]').click()
    expect(onColourDraftChange).toHaveBeenCalledWith('foreground', DEFAULT_SETTINGS.foreground)
  })
})
