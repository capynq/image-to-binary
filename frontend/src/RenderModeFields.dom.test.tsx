/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RenderModeFields } from './RenderModeFields'
import type { RenderModeFieldsProps } from './RenderModeFields'
import { DEFAULT_RAMP } from './processing/protocol'

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

function show(overrides: Partial<RenderModeFieldsProps> = {}): RenderModeFieldsProps {
  const props: RenderModeFieldsProps = {
    mode: 'binary',
    rampDraft: DEFAULT_RAMP,
    onModeChange: vi.fn(),
    onRampDraftChange: vi.fn(),
    ...overrides,
  }
  const target = root ?? createRoot(container)
  act(() => target.render(<RenderModeFields {...props} />))
  return props
}

function radio(value: string): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`)
  if (found === null) throw new Error(`no radio for ${value}`)
  return found
}

function rampInput(): HTMLInputElement | null {
  return container.querySelector<HTMLInputElement>('input[name="ramp"]')
}

function typeInto(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setter?.call(input, value)
  act(() => {
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

describe('render mode fields', () => {
  it('offers classic binary, ASCII and block, with classic binary selected by default', () => {
    show()
    const labels = [...container.querySelectorAll('.mode-option')].map(
      (option) => option.querySelector('span:not(.visually-hidden)')?.textContent,
    )
    expect(labels).toEqual(['Classic binary', 'ASCII', 'Block'])
    expect(radio('binary').checked).toBe(true)
    expect(radio('ascii').checked).toBe(false)
    expect(radio('block').checked).toBe(false)
    expect(rampInput()).toBeNull()
    expect(container.querySelector('legend')?.textContent).toBe('Render mode')
  })

  it('reports mode choices through the native radio buttons', () => {
    const props = show()
    act(() => radio('ascii').click())
    expect(props.onModeChange).toHaveBeenCalledWith('ascii')
    act(() => radio('block').click())
    expect(props.onModeChange).toHaveBeenLastCalledWith('block')
  })

  it('shows the ramp field for ASCII with the default ramp and states the forced sampling', () => {
    show({ mode: 'ascii' })
    const input = rampInput()
    expect(input?.value).toBe(DEFAULT_RAMP)
    expect(input?.getAttribute('aria-invalid')).toBe('false')
    expect(container.textContent).toContain('ASCII uses average sampling.')
    expect(container.querySelector('label[for="' + input?.id + '"]')?.textContent).toBe(
      'Ramp, least ink first',
    )
  })

  it('links an invalid ramp to its error and keeps the error in a polite live region', () => {
    const props = show({ mode: 'ascii', rampDraft: '' })
    const input = rampInput()
    const error = container.querySelector<HTMLElement>('.field-error')
    expect(error?.textContent).toBe('Not applied: ramp must contain at least one character.')
    expect(error?.getAttribute('aria-live')).toBe('polite')
    expect(input?.getAttribute('aria-invalid')).toBe('true')
    const described = input?.getAttribute('aria-describedby')?.split(' ') ?? []
    expect(described).toContain(error?.id)
    expect(described.length).toBe(2)
    expect(props.onRampDraftChange).not.toHaveBeenCalled()
  })

  it('reports each typed ramp and clears the link when the ramp is valid', () => {
    const props = show({ mode: 'ascii' })
    typeInto(rampInput() as HTMLInputElement, 'café')
    expect(props.onRampDraftChange).toHaveBeenLastCalledWith('café')
    show({ mode: 'ascii', rampDraft: '.#@' })
    const input = rampInput()
    const error = container.querySelector<HTMLElement>('.field-error')
    expect(error?.textContent).toBe('')
    expect(input?.getAttribute('aria-invalid')).toBe('false')
    expect(input?.getAttribute('aria-describedby')).toBe(container.querySelector('.field-note')?.id)
  })

  it('names the length failure for a ramp longer than sixty-four characters', () => {
    show({ mode: 'ascii', rampDraft: 'a'.repeat(65) })
    expect(container.querySelector('.field-error')?.textContent).toBe(
      'Not applied: ramp must be at most 64 characters.',
    )
  })

  it('does not treat an invalid draft as a problem outside ASCII mode', () => {
    show({ mode: 'block', rampDraft: '' })
    expect(rampInput()).toBeNull()
    expect(container.querySelector('.field-error')).toBeNull()
  })

  it('keeps every control a native, focusable element', () => {
    show({ mode: 'ascii' })
    for (const value of ['binary', 'ascii', 'block']) {
      const control = radio(value)
      expect(control.disabled).toBe(false)
      act(() => control.focus())
      expect(document.activeElement).toBe(control)
    }
    const input = rampInput()
    act(() => input?.focus())
    expect(document.activeElement).toBe(input)
  })
})

describe('per-option descriptions', () => {
  it('gives each radio its own description, not the current selection', () => {
    show({ mode: 'ascii' })
    const expected: Record<string, string> = {
      binary: 'Binary 0 / 1 output, split at the threshold.',
      ascii: 'ASCII uses average sampling. Brighter cells take later ramp characters.',
      block: 'Block uses average sampling and the Unicode block elements.',
    }
    const described = new Set<string>()
    for (const value of ['binary', 'ascii', 'block']) {
      const id = radio(value).getAttribute('aria-describedby') ?? ''
      expect(id.split(' ')).toHaveLength(1)
      const element = document.getElementById(id)
      expect(element?.textContent).toBe(expected[value])
      described.add(id)
    }
    expect(described.size).toBe(3)
  })
})
