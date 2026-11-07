/** @vitest-environment jsdom */
import { act, createRef } from 'react'
import type { ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ErrorMessage } from './ErrorMessage'
import { InspectorSection } from './InspectorSection'
import { MetadataList } from './MetadataList'
import { Progress } from './Progress'
import { StatusRegion } from './StatusRegion'

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

function bar(): HTMLElement {
  const found = container.querySelector<HTMLElement>('[role="progressbar"]')
  if (found === null) throw new Error('no progressbar')
  return found
}

describe('Progress', () => {
  it('exposes determinate value semantics: valuemin, valuemax, valuenow and valuetext', () => {
    mount(<Progress label="Rendering PNG" fraction={0.42} />)
    expect(bar().getAttribute('aria-valuemin')).toBe('0')
    expect(bar().getAttribute('aria-valuemax')).toBe('100')
    expect(bar().getAttribute('aria-valuenow')).toBe('42')
    expect(bar().getAttribute('aria-valuetext')).toBe('42%')
    expect(container.querySelector('[data-state]')?.getAttribute('data-state')).toBe('determinate')
  })

  it('shows the percentage as text, so the state is not conveyed by the bar alone', () => {
    mount(<Progress label="Rendering PNG" fraction={0.42} />)
    expect(container.querySelector('.progress-percent')?.textContent).toBe('42%')
  })

  it('names the bar with its visible label', () => {
    mount(<Progress label="Rendering PNG" fraction={0.42} />)
    const labelId = bar().getAttribute('aria-labelledby') ?? ''
    expect(document.getElementById(labelId)?.textContent).toBe('Rendering PNG')
  })

  it('clamps out-of-range fractions into 0 to 100 percent', () => {
    mount(<Progress label="Working" fraction={1.7} />)
    expect(bar().getAttribute('aria-valuenow')).toBe('100')
    mount(<Progress label="Working" fraction={-1} />)
    expect(bar().getAttribute('aria-valuenow')).toBe('0')
  })

  it('omits the value for indeterminate work, reporting it as working', () => {
    mount(<Progress label="Importing image…" />)
    expect(bar().hasAttribute('aria-valuenow')).toBe(false)
    expect(bar().hasAttribute('aria-valuemax')).toBe(false)
    expect(container.querySelector('[data-state]')?.getAttribute('data-state')).toBe(
      'indeterminate',
    )
    expect(container.querySelector('.progress-percent')).toBeNull()
  })

  it('offers a cancel action only when one is supplied, and calls it on click', () => {
    const onCancel = vi.fn()
    mount(<Progress label="Working" />)
    expect(container.querySelector('button')).toBeNull()
    mount(<Progress label="Working" onCancel={onCancel} cancelLabel="Cancel export" />)
    const cancel = container.querySelector('button')
    expect(cancel?.textContent).toBe('Cancel export')
    act(() => cancel?.click())
    expect(onCancel).toHaveBeenCalledTimes(1)
  })

  it('gives the cancel action a ref for focus restoration', () => {
    const ref = createRef<HTMLButtonElement>()
    mount(<Progress label="Working" onCancel={() => undefined} cancelRef={ref} />)
    expect(ref.current).toBe(container.querySelector('button'))
  })
})

describe('StatusRegion', () => {
  it('is a polite status live region holding its text', () => {
    mount(<StatusRegion>PNG export ready.</StatusRegion>)
    const region = container.querySelector('[role="status"]')
    expect(region?.getAttribute('aria-live')).toBe('polite')
    expect(region?.textContent).toBe('PNG export ready.')
  })
})

describe('ErrorMessage', () => {
  const long =
    'The image could not be read because the ramp, colour, threshold and cell settings combine into a request that the renderer refuses; choose a smaller cell size or another image.'

  it('is an alert by default and keeps the full message without truncation', () => {
    mount(<ErrorMessage message={long} />)
    const panel = container.querySelector('[role="alert"]')
    expect(panel?.querySelector('.error-text')?.textContent).toBe(long)
    expect(panel?.textContent).not.toContain('…')
  })

  it('drops the alert role when the owner announces the error through a status region', () => {
    mount(<ErrorMessage message="PNG export failed." announce={false} />)
    expect(container.querySelector('[role="alert"]')).toBeNull()
    expect(container.querySelector('.error-panel')).not.toBeNull()
  })

  it('shows guidance beside the message', () => {
    mount(<ErrorMessage message="Failed." guidance="Choose another image." />)
    expect(container.querySelector('.error-guidance')?.textContent).toBe('Choose another image.')
  })

  it('offers a retry action that calls its handler and can be disabled', () => {
    const onRetry = vi.fn()
    mount(<ErrorMessage message="Failed." retry={{ label: 'Retry PNG export', onRetry }} />)
    const retry = container.querySelector('button')
    expect(retry?.textContent).toBe('Retry PNG export')
    act(() => retry?.click())
    expect(onRetry).toHaveBeenCalledTimes(1)
    mount(
      <ErrorMessage
        message="Failed."
        retry={{ label: 'Retry PNG export', onRetry, disabled: true }}
      />,
    )
    expect(container.querySelector('button')?.disabled).toBe(true)
  })

  it('passes the retry ref to the action button', () => {
    const ref = createRef<HTMLButtonElement>()
    mount(
      <ErrorMessage message="Failed." retry={{ label: 'Retry', onRetry: () => undefined, ref }} />,
    )
    expect(ref.current).toBe(container.querySelector('button'))
  })
})

describe('InspectorSection', () => {
  it('is a labelled section whose heading names it', () => {
    mount(
      <InspectorSection title="Metadata">
        <p>Body</p>
      </InspectorSection>,
    )
    const section = container.querySelector('section')
    const headingId = section?.getAttribute('aria-labelledby') ?? ''
    expect(document.getElementById(headingId)?.textContent).toBe('Metadata')
    expect(section?.querySelector('h2')?.hasAttribute('tabindex')).toBe(false)
  })

  it('makes the heading focusable when it is a focus fallback, and applies its class', () => {
    mount(
      <InspectorSection title="Export" className="export-section" focusableHeading>
        <p>Body</p>
      </InspectorSection>,
    )
    expect(container.querySelector('section')?.className).toBe('control-section export-section')
    expect(container.querySelector('h2')?.getAttribute('tabindex')).toBe('-1')
  })
})

describe('MetadataList', () => {
  it('pairs each term with its full value and never drops a long name', () => {
    const name = `${'very-long-file-name-'.repeat(8)}.png`
    mount(
      <MetadataList
        items={[
          { label: 'Filename', value: name },
          { label: 'Dimensions', value: '64 × 40 px' },
        ]}
      />,
    )
    const terms = [...container.querySelectorAll('dt')].map((term) => term.textContent)
    const values = [...container.querySelectorAll('dd')].map((value) => value.textContent)
    expect(terms).toEqual(['Filename', 'Dimensions'])
    expect(values).toEqual([name, '64 × 40 px'])
  })
})
