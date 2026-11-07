/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import App from './App'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

let container: HTMLDivElement
let root: Root | null = null
let controller: StudioController

beforeEach(async () => {
  window.localStorage.setItem('image-to-binary-theme', 'dark')
  container = document.createElement('div')
  document.body.append(container)
  controller = createStudioController({ adapter: createDemoProcessingAdapter() })
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

function toggle(): HTMLButtonElement {
  const found = container.querySelector<HTMLButtonElement>('.inspector-toggle')
  if (found === null) throw new Error('inspector toggle is missing')
  return found
}

describe('App adaptive layout hooks', () => {
  it('exposes the inspector toggle with aria-expanded and a controlled panel', () => {
    expect(toggle().getAttribute('aria-expanded')).toBe('true')
    const panelId = toggle().getAttribute('aria-controls')
    expect(panelId).toBe('inspector-panel')
    expect(container.querySelector(`#${panelId}`)).not.toBeNull()
  })

  it('collapses and expands the inspector without unmounting its content', () => {
    const panel = container.querySelector('#inspector-panel')
    act(() => toggle().click())
    expect(toggle().getAttribute('aria-expanded')).toBe('false')
    expect(container.querySelector('.inspector')?.classList.contains('is-closed')).toBe(true)
    expect(container.querySelector('#inspector-panel')).toBe(panel)
    act(() => toggle().click())
    expect(toggle().getAttribute('aria-expanded')).toBe('true')
    expect(container.querySelector('.inspector')?.classList.contains('is-closed')).toBe(false)
  })

  it('keeps the inspector-open state and the canvas across a re-render, as a resize does', () => {
    act(() => toggle().click())
    const canvas = container.querySelector('.comparison-canvas')
    act(() => root?.render(<App controller={controller} native={null} />))
    expect(toggle().getAttribute('aria-expanded')).toBe('false')
    expect(container.querySelector('.comparison-canvas')).toBe(canvas)
  })

  it('hides the canvas slot exactly while the empty state is shown', () => {
    const slot = container.querySelector<HTMLDivElement>('.canvas-slot')
    expect(slot).not.toBeNull()
    expect(slot?.hidden).toBe(container.querySelector('.empty-canvas') !== null)
    expect(slot?.querySelector('.comparison-canvas')).not.toBeNull()
  })
})
