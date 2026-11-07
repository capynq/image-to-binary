/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ExportPanel } from './ExportPanel'
import type { ExportPanelProps } from './ExportPanel'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

let container: HTMLDivElement
let root: Root | null = null
const extras: HTMLElement[] = []

function panel(focusRequest: number): ExportPanelProps {
  return {
    phase: 'idle',
    canExport: true,
    disabledReason: null,
    progress: 0,
    byteLength: null,
    error: null,
    download: null,
    format: 'png',
    shownFormat: 'png',
    onExport: () => undefined,
    onCancel: () => undefined,
    focusRequest,
  }
}

function render(focusRequest: number): void {
  act(() => root?.render(<ExportPanel {...panel(focusRequest)} />))
}

function exportButton(): HTMLButtonElement {
  const found = container.querySelector<HTMLButtonElement>('.export-section button.button-primary')
  if (found === null) throw new Error('no export control')
  return found
}

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  render(0)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  for (const node of extras.splice(0)) node.remove()
})

describe('ExportPanel settle focus', () => {
  it('moves focus to the phase control when a settle finds focus on body', () => {
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    expect(document.activeElement).toBe(document.body)
    render(1)
    expect(document.activeElement).toBe(exportButton())
  })

  it('leaves focus alone on a control outside the panel', () => {
    const outside = document.createElement('button')
    document.body.append(outside)
    extras.push(outside)
    act(() => outside.focus())
    render(1)
    expect(document.activeElement).toBe(outside)
  })

  it('does nothing without a new focus request', () => {
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    render(0)
    expect(document.activeElement).toBe(document.body)
  })
})
