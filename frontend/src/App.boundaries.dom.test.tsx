/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { RenderModeFields } from './RenderModeFields'
import type { RenderModeFieldsProps } from './RenderModeFields'
import { StudioStatus } from './StudioStatus'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import { MAX_RAMP_LENGTH } from './processing/bounds'
import {
  DEFAULT_RAMP,
  RAMP_EMPTY_MESSAGE,
  RAMP_NOT_PRINTABLE_MESSAGE,
  RAMP_TOO_LONG_MESSAGE,
} from './processing/protocol'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'
import { ErrorMessage, MetadataList } from './ui'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

const LONG_NAME = `${'Long_name_without_spaces_'.repeat(6)}final part ${'with spaces '.repeat(4)}.png`
const LONG_ERROR = `Could not read ${LONG_NAME}: the file is not a supported image, and the decoder stopped before the first pixel row.`

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

function render(element: React.ReactElement): void {
  act(() => root?.render(element))
}

function labelledBy(element: Element): string {
  const id = element.getAttribute('aria-labelledby') ?? ''
  return document.getElementById(id)?.textContent ?? ''
}

function buttonNamed(name: string): HTMLButtonElement {
  const found = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (button) => button.textContent.trim() === name,
  )
  if (found === undefined) throw new Error(`no button named ${name}`)
  return found
}

function canvasReady(): boolean {
  return container.querySelector<HTMLDivElement>('.canvas-slot')?.hidden === false
}

async function waitForCanvasReady(): Promise<void> {
  for (let attempt = 0; attempt < 200 && !canvasReady(); attempt += 1) {
    await act(() => new Promise((resolve) => setTimeout(resolve, 5)))
  }
  if (!canvasReady()) throw new Error('the artwork preview did not finish rendering')
}

describe('long names and full error text', () => {
  it('shows a long file name in full, with its term label kept beside it', () => {
    render(<MetadataList items={[{ label: 'Filename', value: LONG_NAME }]} />)
    const value = container.querySelector('dd')
    expect(value?.textContent).toBe(LONG_NAME)
    expect(container.querySelector('dt')?.textContent).toBe('Filename')
  })

  it('shows the full error text in a polite alert, without truncation', () => {
    render(<ErrorMessage message={LONG_ERROR} guidance="Choose another image." />)
    const panel = container.querySelector('.error-panel')
    expect(panel?.getAttribute('role')).toBe('alert')
    expect(container.querySelector('.error-text')?.textContent).toBe(LONG_ERROR)
  })
})

function rampProps(overrides: Partial<RenderModeFieldsProps>): RenderModeFieldsProps {
  return {
    mode: 'ascii',
    rampDraft: DEFAULT_RAMP,
    onModeChange: vi.fn(),
    onRampDraftChange: vi.fn(),
    ...overrides,
  }
}

function rampField(): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>('input[name="ramp"]')
  if (found === null) throw new Error('ramp field is missing')
  return found
}

function rampError(): string {
  return container.querySelector('.ramp-field .field-error')?.textContent ?? ''
}

describe('ramp extrema', () => {
  it('accepts the minimum length of one character', () => {
    render(<RenderModeFields {...rampProps({ rampDraft: '@' })} />)
    expect(rampError()).toBe('')
    expect(rampField().getAttribute('aria-invalid')).toBe('false')
  })

  it('accepts the maximum length and unusual printable characters', () => {
    const unusual = ` ~"'\\\`{}[]|<>?/;:,.-_+=*&%$#@!()`
      .padEnd(MAX_RAMP_LENGTH, '~')
      .slice(0, MAX_RAMP_LENGTH)
    expect(unusual).toHaveLength(MAX_RAMP_LENGTH)
    render(<RenderModeFields {...rampProps({ rampDraft: unusual })} />)
    expect(rampError()).toBe('')
  })

  it('shows the full too-long message one character past the maximum', () => {
    render(<RenderModeFields {...rampProps({ rampDraft: `${'@'.repeat(MAX_RAMP_LENGTH)}@` })} />)
    expect(rampError()).toBe(`Not applied: ${RAMP_TOO_LONG_MESSAGE}.`)
    expect(rampField().getAttribute('aria-invalid')).toBe('true')
    const describedBy = rampField().getAttribute('aria-describedby') ?? ''
    const errorId = container.querySelector('.ramp-field .field-error')?.id
    expect(errorId).toBeTruthy()
    expect(describedBy.split(' ')).toContain(errorId)
  })

  it('rejects an empty ramp and non-printable characters with their full messages', () => {
    render(<RenderModeFields {...rampProps({ rampDraft: '' })} />)
    expect(rampError()).toBe(`Not applied: ${RAMP_EMPTY_MESSAGE}.`)
    render(<RenderModeFields {...rampProps({ rampDraft: 'é@' })} />)
    expect(rampError()).toBe(`Not applied: ${RAMP_NOT_PRINTABLE_MESSAGE}.`)
    render(<RenderModeFields {...rampProps({ rampDraft: '@\u0001' })} />)
    expect(rampError()).toBe(`Not applied: ${RAMP_NOT_PRINTABLE_MESSAGE}.`)
  })
})

describe('loading and cancel semantics', () => {
  it('labels the progress bar with its visible text and offers a named Cancel', () => {
    const onCancel = vi.fn()
    render(
      <StudioStatus
        busyLabel="Importing image…"
        onCancel={onCancel}
        errorMessage={null}
        guidance=""
        onChooseAnother={vi.fn()}
      />,
    )
    const bar = container.querySelector('[role="progressbar"]')
    expect(bar).not.toBeNull()
    expect(labelledBy(bar as Element)).toBe('Importing image…')
    act(() => buttonNamed('Cancel').click())
    expect(onCancel).toHaveBeenCalledTimes(1)
  })

  it('removes the progress row when work ends', () => {
    render(
      <StudioStatus
        busyLabel="Rendering"
        onCancel={vi.fn()}
        errorMessage={null}
        guidance=""
        onChooseAnother={vi.fn()}
      />,
    )
    expect(container.querySelector('.busy-row')).not.toBeNull()
    render(
      <StudioStatus
        busyLabel={null}
        onCancel={vi.fn()}
        errorMessage={null}
        guidance=""
        onChooseAnother={vi.fn()}
      />,
    )
    expect(container.querySelector('.busy-row')).toBeNull()
  })
})

describe('focus across view switches and viewport commands', () => {
  let controller: StudioController

  beforeEach(async () => {
    window.localStorage.setItem('image-to-binary-theme', 'dark')
    controller = createStudioController({ adapter: createDemoProcessingAdapter() })
    await controller.loadDemo()
    render(<App controller={controller} native={null} />)
  })

  afterEach(() => {
    controller.dispose()
    window.localStorage.removeItem('image-to-binary-theme')
  })

  it('keeps focus on the view choice after switching to Split', () => {
    const split = container.querySelector<HTMLInputElement>('.view-toggle input[value="split"]')
    expect(split).not.toBeNull()
    act(() => split?.focus())
    act(() => split?.click())
    expect(document.activeElement).toBe(split)
    expect(container.querySelector('#canvas-heading')?.textContent).toBe('Split comparison')
  })

  it('keeps focus on a viewport command after it runs', async () => {
    const artwork = container.querySelector<HTMLInputElement>('.view-toggle input[value="artwork"]')
    act(() => artwork?.click())
    await waitForCanvasReady()
    const zoomIn = [
      ...container.querySelectorAll<HTMLButtonElement>('.viewport-controls button'),
    ].find((button) => button.textContent.trim() === 'Zoom in')
    expect(zoomIn).toBeDefined()
    act(() => zoomIn?.focus())
    act(() => zoomIn?.click())
    expect(document.activeElement?.textContent?.trim()).toBe('Zoom in')
    expect(document.activeElement?.tagName).toBe('BUTTON')
    expect(document.activeElement).not.toBe(document.body)
  })
})
