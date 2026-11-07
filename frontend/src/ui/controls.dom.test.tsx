/** @vitest-environment jsdom */
import { act, useState } from 'react'
import type { ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Button } from './Button'
import { Checkbox } from './Checkbox'
import { ColourField } from './ColourField'
import { SegmentedChoice } from './SegmentedChoice'
import { SliderField } from './SliderField'
import { TextField } from './TextField'

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

function mount(element: ReactElement): void {
  const target = root ?? createRoot(container)
  act(() => target.render(element))
}

function typeInto(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  setter?.call(input, value)
  act(() => {
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function byId(id: string): HTMLElement {
  const found = document.getElementById(id)
  if (found === null) throw new Error(`no element ${id}`)
  return found
}

function describedText(element: Element): string {
  return (element.getAttribute('aria-describedby') ?? '')
    .split(' ')
    .filter((id) => id !== '')
    .map((id) => byId(id).textContent ?? '')
    .join(' | ')
}

describe('Button', () => {
  it('renders a native button with the variant, wide and 44 px classes', () => {
    mount(
      <Button variant="primary" wide>
        Export
      </Button>,
    )
    const button = container.querySelector('button')
    expect(button?.type).toBe('button')
    expect(button?.className).toBe('button button-primary button-wide')
  })

  it('fires its click only while enabled', () => {
    const onClick = vi.fn()
    mount(<Button onClick={onClick}>Go</Button>)
    act(() => container.querySelector('button')?.click())
    expect(onClick).toHaveBeenCalledTimes(1)
    mount(
      <Button disabled onClick={onClick}>
        Go
      </Button>,
    )
    act(() => container.querySelector('button')?.click())
    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('marks busy work with aria-busy and stays enabled, so focus is not lost', () => {
    mount(<Button busy>Rendering…</Button>)
    const button = container.querySelector('button')
    expect(button?.getAttribute('aria-busy')).toBe('true')
    expect(button?.disabled).toBe(false)
    expect(button?.classList.contains('is-busy')).toBe(true)
  })

  it('forwards its ref to the button element', () => {
    const ref = { current: null as HTMLButtonElement | null }
    mount(<Button ref={ref}>Focus me</Button>)
    expect(ref.current).toBe(container.querySelector('button'))
  })
})

describe('SegmentedChoice', () => {
  const options = [
    { value: 'a', label: 'Alpha', description: 'First option.' },
    { value: 'b', label: 'Beta', description: 'Second option.' },
    { value: 'c', label: 'Gamma', disabled: true },
  ] as const

  function show(value: 'a' | 'b' | 'c', onChange = vi.fn(), legendHidden = false) {
    mount(
      <SegmentedChoice
        legend="Pick one"
        legendHidden={legendHidden}
        options={options}
        value={value}
        onChange={onChange}
      />,
    )
    return onChange
  }

  function radio(value: string): HTMLInputElement {
    const found = container.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`)
    if (found === null) throw new Error(`no radio ${value}`)
    return found
  }

  it('is a fieldset whose legend names the group, with one native radio per option', () => {
    show('a')
    expect(container.querySelector('fieldset')?.querySelector('legend')?.textContent).toBe(
      'Pick one',
    )
    const radios = [...container.querySelectorAll<HTMLInputElement>('input[type="radio"]')]
    expect(radios).toHaveLength(3)
    expect(new Set(radios.map((input) => input.name)).size).toBe(1)
  })

  it('checks exactly the option whose value is selected', () => {
    show('b')
    expect(radio('a').checked).toBe(false)
    expect(radio('b').checked).toBe(true)
  })

  it('gives the selected option a non-colour cue: a check state and a heavier border class', () => {
    show('b')
    const cues = [...container.querySelectorAll<HTMLElement>('.choice-cue')]
    expect(cues.map((cue) => cue.dataset.state)).toEqual(['unselected', 'selected', 'unselected'])
    const selected = radio('b').closest('label')
    expect(selected?.classList.contains('is-selected')).toBe(true)
    expect(selected?.querySelector('.choice-cue')?.getAttribute('aria-hidden')).toBe('true')
  })

  it('draws the check mark only on the selected cue, so the state survives without colour', () => {
    show('b')
    const cues = [...container.querySelectorAll<HTMLElement>('.choice-cue')]
    expect(cues.map((cue) => cue.querySelector('svg') !== null)).toEqual([false, true, false])
  })

  it('reports a change through onChange when another option is clicked', () => {
    const onChange = show('a')
    act(() => radio('b').click())
    expect(onChange).toHaveBeenCalledWith('b')
  })

  it('disables an option that is unavailable and does not select it on click', () => {
    const onChange = show('a')
    expect(radio('c').disabled).toBe(true)
    act(() => radio('c').click())
    expect(onChange).not.toHaveBeenCalled()
  })

  it('links each described option to its description, outside the option label', () => {
    show('a')
    const alpha = radio('a')
    expect(describedText(alpha)).toBe('First option.')
    const label = alpha.closest('label')
    expect(label?.textContent).toBe('Alpha')
    expect(label?.contains(byId(alpha.getAttribute('aria-describedby') ?? ''))).toBe(false)
    expect(radio('c').hasAttribute('aria-describedby')).toBe(false)
  })

  it('hides a visually hidden legend without removing it from the group', () => {
    show('a', vi.fn(), true)
    expect(container.querySelector('legend')?.className).toBe('visually-hidden')
    expect(container.querySelector('legend')?.textContent).toBe('Pick one')
  })
})

describe('SliderField', () => {
  function show(value: number, onChange = vi.fn()) {
    mount(
      <SliderField
        id="size"
        name="size"
        label="Cell size"
        unit="px"
        min={5}
        max={30}
        value={value}
        onChange={onChange}
      />,
    )
    return onChange
  }

  it('keeps the label and the output as siblings, never nesting the output in the label', () => {
    show(12)
    const label = container.querySelector('label')
    const output = container.querySelector('output')
    expect(label?.querySelector('output')).toBeNull()
    expect(label?.textContent).toBe('Cell size')
    expect(output?.textContent).toBe('12px')
    expect(output?.getAttribute('for')).toBe('size')
    expect(label?.getAttribute('for')).toBe('size')
  })

  it('keeps a stable accessible name that does not change with the value', () => {
    show(12)
    const first = container.querySelector('label')?.textContent
    mount(
      <SliderField
        id="size"
        name="size"
        label="Cell size"
        unit="px"
        min={5}
        max={30}
        value={20}
        onChange={vi.fn()}
      />,
    )
    expect(container.querySelector('label')?.textContent).toBe(first)
  })

  it('exposes the value with its unit through aria-valuetext', () => {
    show(12)
    const input = container.querySelector<HTMLInputElement>('input[type="range"]')
    expect(input?.getAttribute('aria-valuetext')).toBe('12px')
    expect(input?.min).toBe('5')
    expect(input?.max).toBe('30')
  })

  it('passes the numeric value of a change to onChange', () => {
    const onChange = show(12)
    const input = container.querySelector<HTMLInputElement>('input[type="range"]')
    if (input === null) throw new Error('no slider')
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    setter?.call(input, '18')
    act(() => {
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    expect(onChange).toHaveBeenLastCalledWith(18)
  })
})

function input(): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>('input[name="ramp"]')
  if (found === null) throw new Error('no text input')
  return found
}

/** A parent that owns the draft, as the studio does, so each keystroke re-renders the field. */
function ControlledRamp({ error }: { readonly error: string }) {
  const [value, setValue] = useState('abc')
  return (
    <TextField id="ramp" name="ramp" label="Ramp" value={value} onChange={setValue} error={error} />
  )
}

function mountControlledRamp(error: string): void {
  mount(<ControlledRamp error={error} />)
}

describe('TextField', () => {
  function show(error: string | null, describedBy?: string) {
    const onChange = vi.fn()
    mount(
      <div>
        <p id="note">Visible note.</p>
        <TextField
          id="ramp"
          name="ramp"
          label="Ramp"
          value="abc"
          onChange={onChange}
          error={error}
          describedBy={describedBy}
        />
      </div>,
    )
    return onChange
  }

  it('is valid with no error: aria-invalid is false and only the note is described', () => {
    show(null, 'note')
    expect(input().getAttribute('aria-invalid')).toBe('false')
    expect(input().getAttribute('aria-describedby')).toBe('note')
  })

  it('marks an error with aria-invalid and links the error text as a description', () => {
    show('Not applied: too long.', 'note')
    expect(input().getAttribute('aria-invalid')).toBe('true')
    expect(describedText(input())).toBe('Visible note. | Not applied: too long.')
  })

  it('keeps the full error text in a polite live region', () => {
    show('Not applied: ramp must be at most 64 characters.')
    const error = container.querySelector('.field-error')
    expect(error?.textContent).toBe('Not applied: ramp must be at most 64 characters.')
    expect(error?.getAttribute('aria-live')).toBe('polite')
  })

  it('names its label through htmlFor', () => {
    show(null)
    expect(container.querySelector('label')?.getAttribute('for')).toBe('ramp')
    expect(input().id).toBe('ramp')
  })

  it('reports each keystroke through onChange', () => {
    const onChange = show(null)
    typeInto(input(), 'abcd')
    expect(onChange).toHaveBeenLastCalledWith('abcd')
  })

  it('does not touch the error region while the same error persists across keystrokes', () => {
    mountControlledRamp('Not applied: bad.')
    const region = container.querySelector('.field-error')
    if (region === null) throw new Error('no error region')
    const records: MutationRecord[] = []
    const observer = new MutationObserver((changes) => records.push(...changes))
    observer.observe(container, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
    })
    typeInto(input(), 'abcd')
    typeInto(input(), 'abcde')
    records.push(...observer.takeRecords())
    observer.disconnect()
    expect(input().value).toBe('abcde')
    expect(container.querySelector('.field-error')).toBe(region)
    const errorChanges = records.filter(
      (record) =>
        record.target === region ||
        (record.type === 'characterData' && record.target.parentNode === region) ||
        (record.type === 'childList' &&
          [...record.addedNodes, ...record.removedNodes].some(
            (node) => node instanceof Element && node.matches('.field-error'),
          )),
    )
    expect(errorChanges).toHaveLength(0)
  })

  it('updates the error region when its text changes', () => {
    show('Not applied: bad.')
    const records: MutationRecord[] = []
    const observer = new MutationObserver((changes) => records.push(...changes))
    observer.observe(container, { subtree: true, childList: true, characterData: true })
    mount(
      <TextField
        id="ramp"
        name="ramp"
        label="Ramp"
        value="x"
        onChange={vi.fn()}
        error="Not applied: other."
      />,
    )
    records.push(...observer.takeRecords())
    observer.disconnect()
    expect(container.querySelector('.field-error')?.textContent).toBe('Not applied: other.')
    expect(records.length).toBeGreaterThan(0)
  })
})

describe('ColourField', () => {
  function show(error: string | null, onPickerOpen = vi.fn()) {
    const handlers = { onPick: vi.fn(), onDraftChange: vi.fn(), onPickerOpen }
    mount(
      <ColourField
        label="Foreground colour"
        pickerName="foregroundPicker"
        hexName="foreground"
        accepted="#00ff00"
        draft="#00ff0"
        error={error}
        {...handlers}
      />,
    )
    return handlers
  }

  it('pairs a labelled picker with a labelled hex field', () => {
    show(null)
    const picker = container.querySelector<HTMLInputElement>('input[type="color"]')
    const hex = container.querySelector<HTMLInputElement>('input[name="foreground"]')
    expect(picker?.value).toBe('#00ff00')
    expect(hex?.value).toBe('#00ff0')
    expect(container.querySelector(`label[for="${picker?.id}"]`)?.textContent).toBe(
      'Foreground colour',
    )
    expect(container.querySelector(`label[for="${hex?.id}"]`)?.textContent).toBe(
      'Foreground colour hex',
    )
  })

  it('describes an invalid hex draft with aria-invalid and the full error text', () => {
    show('Not applied: colour must be #RRGGBB.')
    const hex = container.querySelector<HTMLInputElement>('input[name="foreground"]')
    expect(hex?.getAttribute('aria-invalid')).toBe('true')
    expect(describedText(hex as HTMLInputElement)).toContain('Not applied: colour must be #RRGGBB.')
  })

  it('reports a typed draft and a picked colour through their own handlers', () => {
    const handlers = show(null)
    const hex = container.querySelector<HTMLInputElement>('input[name="foreground"]')
    if (hex === null) throw new Error('no hex field')
    typeInto(hex, '#112233')
    expect(handlers.onDraftChange).toHaveBeenLastCalledWith('#112233')
    expect(handlers.onPick).not.toHaveBeenCalled()
  })

  it('resets the draft when the picker opens', () => {
    const onPickerOpen = vi.fn()
    show(null, onPickerOpen)
    act(() => container.querySelector<HTMLInputElement>('input[type="color"]')?.click())
    expect(onPickerOpen).toHaveBeenCalledTimes(1)
  })
})

describe('Checkbox', () => {
  it('wraps a native checkbox in its label and reports the checked state', () => {
    const onChange = vi.fn()
    mount(<Checkbox name="invert" label="Invert tones" checked={false} onChange={onChange} />)
    const box = container.querySelector<HTMLInputElement>('input[type="checkbox"]')
    expect(container.querySelector('label')?.textContent).toBe('Invert tones')
    expect(box?.checked).toBe(false)
    act(() => box?.click())
    expect(onChange).toHaveBeenCalledWith(true)
  })
})
