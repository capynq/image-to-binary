/** @vitest-environment jsdom */
import { StrictMode, act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { createDownloadOwner } from './downloadOwner'
import type { DownloadLink } from './downloadOwner'
import { ExportPanel } from './ExportPanel'
import type { ExportPanelProps } from './ExportPanel'
import { artifactOf, countingUrlApi, textArtifactOf } from './exportTestSupport'
import type { ExportFormat } from '../processing/types'

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

function textLinkFor(size: number, filename: string): DownloadLink {
  const link = createDownloadOwner(countingUrlApi()).show(textArtifactOf(size), filename)
  if (link === null) throw new Error('A link was expected.')
  return link
}

function formatRadio(value: ExportFormat): HTMLInputElement {
  const found = container.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`)
  if (found === null) throw new Error(`No format radio ${value}.`)
  return found
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

describe('ExportPanel saved export', () => {
  it('shows the saved file and size in place of the download link', () => {
    show({ phase: 'ready', byteLength: 2048, savedName: 'chosen.png' })
    const text = container.textContent ?? ''
    expect(text).toContain('Saved chosen.png, 2.0 KiB.')
    expect(text).not.toContain('Encoded PNG')
    expect(container.querySelector('a')).toBeNull()
    expect(control('Export again')).toBeInstanceOf(HTMLButtonElement)
  })

  it('moves focus to Export again, since there is no download link to receive it', () => {
    show({ phase: 'running' })
    control('Cancel export').focus()
    show({ phase: 'ready', byteLength: 2048, savedName: 'chosen.png' })
    expect(document.activeElement).toBe(control('Export again'))
  })

  it('announces the saved name and size once', () => {
    show({ phase: 'running' })
    show({ phase: 'ready', byteLength: 2048, savedName: 'chosen.png' })
    expect(liveText()).toBe('Saved chosen.png, 2.0 KiB.')
    expect(container.querySelectorAll('[role="status"]')).toHaveLength(1)
  })

  it('a cancelled save dialog leaves the panel idle with no failure text', () => {
    show({ phase: 'running' })
    show({ phase: 'idle' })
    expect(liveText()).toBe('PNG export cancelled.')
    expect(container.querySelector('.error-panel')).toBeNull()
    expect(control('Export PNG')).toBeInstanceOf(HTMLButtonElement)
  })
})

describe('ExportPanel format choice', () => {
  it('lists PNG and text as radios in one Format group, with PNG chosen by default', () => {
    show({ format: 'png', onFormatChange: noop })
    const group = container.querySelector('fieldset')
    expect(group?.querySelector('legend')?.textContent).toBe('Format')
    const radios = [...container.querySelectorAll<HTMLInputElement>('input[type="radio"]')]
    expect(radios.map((radio) => radio.value)).toEqual(['png', 'text'])
    expect(new Set(radios.map((radio) => radio.name)).size).toBe(1)
    expect(formatRadio('png').checked).toBe(true)
    expect(control('Export PNG')).toBeTruthy()
  })

  it('reports the chosen format and names the export button for it', () => {
    const chosen: ExportFormat[] = []
    show({ format: 'png', onFormatChange: (format) => chosen.push(format) })
    act(() => formatRadio('text').click())
    expect(chosen).toEqual(['text'])
    show({ format: 'text', onFormatChange: noop })
    expect(control('Export text')).toBeTruthy()
    expect(formatRadio('text').checked).toBe(true)
    expect(container.textContent).toContain('Text uses the same grid and settings as PNG')
  })

  it('hides the format choice while an export runs or prepares', () => {
    show({ phase: 'running', format: 'text', shownFormat: 'text', onFormatChange: noop })
    expect(container.querySelector('fieldset')).toBeNull()
    show({ phase: 'preparing', format: 'text', shownFormat: 'text', onFormatChange: noop })
    expect(container.querySelector('fieldset')).toBeNull()
  })

  it('keeps keyboard focus on the chosen format radio, so choosing does not move focus away', () => {
    show({ format: 'png', onFormatChange: noop })
    formatRadio('text').focus()
    act(() => formatRadio('text').click())
    show({ format: 'text', onFormatChange: noop })
    expect(document.activeElement).toBe(formatRadio('text'))
  })

  it('keeps the export section without a format control when none is wired', () => {
    show({})
    expect(container.querySelector('fieldset')).toBeNull()
    expect(control('Export PNG')).toBeTruthy()
  })

  it('moves focus to Cancel for a text render and to Download text when it is ready', () => {
    show({ format: 'text', onFormatChange: noop })
    control('Export text').focus()
    show({ phase: 'running', format: 'text', shownFormat: 'text', progress: 0.1 })
    expect(document.activeElement).toBe(control('Cancel export'))
    show({
      phase: 'ready',
      format: 'text',
      shownFormat: 'text',
      byteLength: 2048,
      download: textLinkFor(2048, 'grid-binary.txt'),
    })
    expect(document.activeElement).toBe(control('Download text'))
  })

  it('moves focus to Retry text export when a text export fails', () => {
    show({ phase: 'running', format: 'text', shownFormat: 'text' })
    control('Cancel export').focus()
    show({ phase: 'failed', format: 'text', shownFormat: 'text', error: null })
    expect(document.activeElement).toBe(control('Retry text export'))
    expect(container.querySelector('.error-text')?.textContent).toBe('The text export failed.')
  })

  it('invokes Cancel for a text render and speaks the text cancellation', () => {
    let cancelled = 0
    show({
      phase: 'running',
      format: 'text',
      shownFormat: 'text',
      onCancel: () => (cancelled += 1),
    })
    act(() => control('Cancel export').click())
    expect(cancelled).toBe(1)
    show({ phase: 'idle', format: 'text', shownFormat: 'text', onFormatChange: noop })
    expect(liveText()).toBe('Text export cancelled.')
  })

  it('speaks the text render, the ready size and a saved text file', () => {
    show({ phase: 'running', format: 'text', shownFormat: 'text' })
    expect(liveText()).toBe('Rendering text export.')
    show({
      phase: 'ready',
      format: 'text',
      shownFormat: 'text',
      byteLength: 2048,
      download: textLinkFor(2048, 'grid-binary.txt'),
    })
    expect(liveText()).toBe('Text export ready, 2.0 KiB.')
    show({ phase: 'idle', format: 'text', shownFormat: 'text', onFormatChange: noop })
    show({
      phase: 'ready',
      format: 'text',
      shownFormat: 'text',
      byteLength: 2048,
      savedName: 'grid.txt',
    })
    expect(liveText()).toBe('Saved grid.txt, 2.0 KiB.')
  })

  it('offers a Download text link with the .txt filename', () => {
    show({
      phase: 'ready',
      format: 'text',
      shownFormat: 'text',
      byteLength: 2048,
      download: textLinkFor(2048, 'grid-binary.txt'),
    })
    const link = control('Download text') as HTMLAnchorElement
    expect(link.getAttribute('download')).toBe('grid-binary.txt')
    expect(link.getAttribute('href')).toMatch(/^blob:/)
  })
})
