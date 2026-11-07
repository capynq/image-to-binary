/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { createDemoProcessingAdapter } from './processing/demoAdapter'
import { DEFAULT_FOREGROUND } from './processing/protocol'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'
import type { RenderStyleFieldsProps } from './RenderStyleFields'

vi.mock('./RenderStyleFields', async (importOriginal) => {
  const original = await importOriginal<typeof import('./RenderStyleFields')>()
  return {
    ...original,
    RenderStyleFields: (props: RenderStyleFieldsProps) => (
      <button
        type="button"
        data-testid="forward-invalid"
        onClick={() => props.onColourChange('foreground', '#zz0000')}
      >
        forward invalid
      </button>
    ),
  }
})

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
  vi.restoreAllMocks()
})

describe('App colour guard', () => {
  it('never hands an invalid colour to the controller, even when a child forwards one', () => {
    const update = vi.spyOn(controller, 'updateSettings')
    const forward = container.querySelector<HTMLButtonElement>('[data-testid="forward-invalid"]')
    if (forward === null) throw new Error('stub control is missing')
    act(() => forward.click())
    const invalid = update.mock.calls.filter(([next]) => next.foreground === '#zz0000')
    expect(invalid).toEqual([])
    expect(controller.getSnapshot().settings.foreground).toBe(DEFAULT_FOREGROUND)
  })
})
