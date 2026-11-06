/** @vitest-environment jsdom */
import { StrictMode, act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { createDownloadOwner } from './downloadOwner'
import type { DownloadLink } from './downloadOwner'
import { ExportPanel } from './ExportPanel'
import type { ExportPanelProps } from './ExportPanel'
import { artifactOf, countingUrlApi } from './exportTestSupport'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

const noop = () => undefined
const MESSAGE =
  'The PNG encoder stopped after 3 of 9 chunks. The encoded output would exceed the browser limit.'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  unmount()
  container.remove()
})

function unmount(): void {
  if (root === null) return
  const mounted = root
  root = null
  act(() => mounted.unmount())
}

function show(overrides: Partial<ExportPanelProps>, strict = false): void {
  const element = (
    <ExportPanel
      phase="idle"
      canExport
      disabledReason={null}
      progress={0}
      byteLength={null}
      error={null}
      download={null}
      onExport={noop}
      onCancel={noop}
      {...overrides}
    />
  )
  if (root === null) root = createRoot(container)
  const target = root
  act(() => target.render(strict ? <StrictMode>{element}</StrictMode> : element))
}

function control(label: string): HTMLElement {
  const found = [...container.querySelectorAll<HTMLElement>('button, a')].find(
    (element) => element.textContent?.trim() === label,
  )
  if (found === undefined) throw new Error(`No control labelled ${label}.`)
  return found
}

function liveText(): string {
  return container.querySelector('.export-status')?.textContent ?? ''
}

function linkFor(size: number, filename: string): DownloadLink {
  const link = createDownloadOwner(countingUrlApi()).show(artifactOf(size), filename)
  if (link === null) throw new Error('A link was expected.')
  return link
}

describe('ExportPanel focus', () => {
  it('moves focus to Cancel when the render starts', () => {
    show({})
    control('Export PNG').focus()
    show({ phase: 'running', progress: 0.1 })
    expect(document.activeElement).toBe(control('Cancel export'))
  })

  it('returns focus to Export after Cancel', () => {
    show({ phase: 'running' })
    control('Cancel export').focus()
    show({ phase: 'idle' })
    expect(document.activeElement).toBe(control('Export PNG'))
  })

  it('moves focus to Download when the export is ready', () => {
    show({ phase: 'running' })
    control('Cancel export').focus()
    show({ phase: 'ready', byteLength: 2048, download: linkFor(2048, 'a-binary.png') })
    expect(document.activeElement).toBe(control('Download PNG'))
  })

  it('moves focus to Retry when the export fails', () => {
    show({ phase: 'running' })
    control('Cancel export').focus()
    show({ phase: 'failed', error: MESSAGE })
    expect(document.activeElement).toBe(control('Retry PNG export'))
  })

  it('leaves focus alone when it sits outside the export panel', () => {
    const outside = document.createElement('button')
    document.body.append(outside)
    outside.focus()
    show({ phase: 'running' })
    expect(document.activeElement).toBe(outside)
    outside.remove()
  })
})

describe('ExportPanel announcements', () => {
  it('speaks the start, keeps the text through the download step, and speaks the ready size', () => {
    show({})
    show({ phase: 'running', progress: 0.5 })
    expect(liveText()).toBe('Rendering PNG export.')
    show({ phase: 'preparing', progress: 1 })
    expect(liveText()).toBe('Rendering PNG export.')
    show({ phase: 'ready', byteLength: 2048, download: linkFor(2048, 'a-binary.png') })
    expect(liveText()).toBe('PNG export ready, 2.0 KiB.')
  })

  it('announces a running-to-idle supersession or cancel as a cancellation', () => {
    show({ phase: 'running' })
    show({ phase: 'idle' })
    expect(liveText()).toBe('PNG export cancelled.')
  })

  it('stays silent when a ready export returns to idle', () => {
    show({ phase: 'ready', byteLength: 2048, download: linkFor(2048, 'a-binary.png') })
    show({ phase: 'idle' })
    expect(liveText()).toBe('')
  })

  it('announces a failure with the full error message and no alert role', () => {
    show({ phase: 'running' })
    show({ phase: 'failed', error: MESSAGE })
    expect(liveText()).toBe(`PNG export failed. ${MESSAGE}`)
    expect(container.querySelector('.error-text')?.textContent).toBe(MESSAGE)
    expect(container.querySelectorAll('[role="alert"]')).toHaveLength(0)
    expect(container.querySelectorAll('[role="status"]')).toHaveLength(1)
  })
})
