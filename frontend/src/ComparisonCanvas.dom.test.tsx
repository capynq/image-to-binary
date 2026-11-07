/** @vitest-environment jsdom */
import { act } from 'react'
import type { ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ComparisonCanvas } from './ComparisonCanvas'
import type { ComparisonCanvasProps } from './ComparisonCanvas'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

const VIEWPORT_WIDTH = 400
const VIEWPORT_HEIGHT = 300

let container: HTMLDivElement
let root: Root | null = null
const originalClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth')
const originalClientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight')

/** A viewport inside a `hidden` ancestor is display:none in a browser and measures zero. */
function isLaidOut(element: HTMLElement): boolean {
  return element.classList.contains('comparison-viewport') && element.closest('[hidden]') === null
}

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
    configurable: true,
    get(this: HTMLElement) {
      return isLaidOut(this) ? VIEWPORT_WIDTH : 0
    },
  })
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get(this: HTMLElement) {
      return isLaidOut(this) ? VIEWPORT_HEIGHT : 0
    },
  })
})

afterEach(() => {
  if (root !== null) {
    const mounted = root
    root = null
    act(() => mounted.unmount())
  }
  container.remove()
  if (originalClientWidth !== undefined) {
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', originalClientWidth)
  }
  if (originalClientHeight !== undefined) {
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', originalClientHeight)
  }
})

const SOURCE_TOKEN = { name: 'sample' }

function props(overrides: Partial<ComparisonCanvasProps> = {}): ComparisonCanvasProps {
  return {
    view: 'split',
    source: { width: 252, height: 192 },
    sourceToken: SOURCE_TOKEN,
    original: { url: 'blob:original', width: 252, height: 192 },
    artwork: { url: 'blob:artwork', width: 256, height: 192 },
    originalAlt: 'Original image',
    artworkAlt: 'Binary artwork preview',
    ...overrides,
  }
}

function show(element: ReactElement): void {
  root ??= createRoot(container)
  const target = root
  act(() => target.render(element))
}

function viewport(): HTMLDivElement {
  const found = container.querySelector<HTMLDivElement>('.comparison-viewport')
  if (found === null) throw new Error('no viewport')
  return found
}

function slider(): HTMLElement {
  const found = container.querySelector<HTMLElement>('[role="slider"]')
  if (found === null) throw new Error('no slider')
  return found
}

function readout(): string {
  return container.querySelector('.zoom-readout')?.textContent ?? ''
}

function buttonNamed(label: string): HTMLButtonElement {
  const found = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  if (found === undefined) throw new Error(`no button ${label}`)
  return found
}

function press(target: Element, key: string, init: KeyboardEventInit = {}): void {
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init }),
    )
  })
}

function wheel(target: Element, init: WheelEventInit): WheelEvent {
  const event = new WheelEvent('wheel', { bubbles: true, cancelable: true, ...init })
  act(() => {
    target.dispatchEvent(event)
  })
  return event
}

describe('labelling and images', () => {
  it('labels the canvas region and gives each image alt text naming its view', () => {
    show(<ComparisonCanvas {...props()} />)
    expect(container.querySelector('section')?.getAttribute('aria-label')).toBe('Preview canvas')
    const images = [...container.querySelectorAll('img')]
    expect(images.map((image) => image.getAttribute('alt'))).toEqual([
      'Original image',
      'Binary artwork preview',
    ])
  })

  it('shows one image in the original and artwork views', () => {
    show(<ComparisonCanvas {...props({ view: 'original' })} />)
    expect(container.querySelectorAll('img')).toHaveLength(1)
    expect(container.querySelector('[role="slider"]')).toBeNull()
  })

  it('spans the same source extent as the original in a split, so the halves share one scale', () => {
    show(<ComparisonCanvas {...props({ view: 'split' })} />)
    const [original, artwork] = [...container.querySelectorAll<HTMLImageElement>('img')]
    const originalWidth = Number.parseFloat(original?.style.width ?? '')
    const artworkWidth = Number.parseFloat(artwork?.style.width ?? '')
    expect(artworkWidth / originalWidth).toBeCloseTo(1, 6)
  })

  it('shows the readout as text, starting fitted', () => {
    show(<ComparisonCanvas {...props()} />)
    expect(readout().startsWith('Fit, ')).toBe(true)
  })
})

describe('slider semantics', () => {
  it('is a labelled horizontal role=slider with its value and text', () => {
    show(<ComparisonCanvas {...props()} />)
    expect(slider().getAttribute('aria-label')).toBe('Split position')
    expect(slider().getAttribute('aria-orientation')).toBe('horizontal')
    expect(slider().getAttribute('aria-valuemin')).toBe('0')
    expect(slider().getAttribute('aria-valuemax')).toBe('100')
    expect(slider().getAttribute('aria-valuenow')).toBe('50')
    expect(slider().getAttribute('aria-valuetext')).toBe('50% original, 50% artwork')
    expect(slider().tabIndex).toBe(0)
  })

  it('moves with arrow keys, Shift for ten steps, Home and End, and reports each value', () => {
    show(<ComparisonCanvas {...props()} />)
    press(slider(), 'ArrowRight')
    expect(slider().getAttribute('aria-valuenow')).toBe('51')
    press(slider(), 'ArrowLeft', { shiftKey: true })
    expect(slider().getAttribute('aria-valuenow')).toBe('41')
    press(slider(), 'End')
    expect(slider().getAttribute('aria-valuetext')).toBe('100% original, 0% artwork')
    press(slider(), 'Home')
    expect(slider().getAttribute('aria-valuenow')).toBe('0')
  })

  it('keeps focus on the divider while it moves, never on body', () => {
    show(<ComparisonCanvas {...props()} />)
    act(() => slider().focus())
    press(slider(), 'ArrowRight')
    expect(document.activeElement).toBe(slider())
  })

  it('does not treat other keys as split changes', () => {
    show(<ComparisonCanvas {...props()} />)
    press(slider(), 'x')
    expect(slider().getAttribute('aria-valuenow')).toBe('50')
  })
})

describe('viewport keyboard and focus', () => {
  it('zooms with plus and minus, sets 100% with 1, and refits with 0, when the viewport has focus', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    act(() => viewport().focus())
    press(viewport(), '+')
    expect(readout()).not.toMatch(/^Fit/)
    press(viewport(), '1')
    expect(readout()).toBe('100%')
    press(viewport(), '-')
    expect(readout()).toBe('80%')
    press(viewport(), '0')
    expect(readout().startsWith('Fit, ')).toBe(true)
    expect(document.activeElement).toBe(viewport())
  })

  it('ignores the viewport keys when a child, such as the divider, has the focus', () => {
    show(<ComparisonCanvas {...props()} />)
    const before = readout()
    press(slider(), '+')
    expect(readout()).toBe(before)
  })

  it('leaves modified keys alone so the browser and menu keep their shortcuts', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    const before = readout()
    press(viewport(), '+', { ctrlKey: true })
    expect(readout()).toBe(before)
  })

  it('keeps focus on a zoom button after it is pressed, even when the zoom is at a limit', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    for (let index = 0; index < 40; index += 1) {
      act(() => buttonNamed('Zoom in').click())
    }
    act(() => buttonNamed('Zoom in').focus())
    act(() => buttonNamed('Zoom in').click())
    expect(document.activeElement).toBe(buttonNamed('Zoom in'))
    expect(buttonNamed('Zoom in').disabled).toBe(false)
  })

  it('has zoom controls named by their visible text', () => {
    show(<ComparisonCanvas {...props()} />)
    for (const label of ['Zoom out', 'Zoom in', 'Fit', '100%']) {
      expect(buttonNamed(label).textContent?.trim()).toBe(label)
    }
  })
})

describe('wheel', () => {
  it('leaves page scroll alone when the image fits and the wheel is plain', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    const event = wheel(viewport(), { deltaY: 80 })
    expect(event.defaultPrevented).toBe(false)
  })

  it('zooms around the pointer with Ctrl and suppresses the page zoom', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    const before = readout()
    const event = wheel(viewport(), { deltaY: -40, ctrlKey: true, clientX: 200, clientY: 150 })
    expect(event.defaultPrevented).toBe(true)
    expect(readout()).not.toBe(before)
  })

  it('pans a zoomed image on a plain wheel and suppresses scroll only when the image moves', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    for (let index = 0; index < 4; index += 1) act(() => buttonNamed('Zoom in').click())
    const event = wheel(viewport(), { deltaY: 40 })
    expect(event.defaultPrevented).toBe(true)
  })

  it('lets scroll pass at a pan limit, so the page keeps scrolling', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    for (let index = 0; index < 4; index += 1) act(() => buttonNamed('Zoom in').click())
    wheel(viewport(), { deltaY: 100_000 })
    const atLimit = wheel(viewport(), { deltaY: 100_000 })
    expect(atLimit.defaultPrevented).toBe(false)
  })
})

describe('refit rules', () => {
  it('refits when a new source arrives', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    act(() => buttonNamed('100%').click())
    expect(readout()).toBe('100%')
    show(<ComparisonCanvas {...props({ view: 'artwork', sourceToken: { name: 'next' } })} />)
    expect(readout().startsWith('Fit, ')).toBe(true)
  })

  it('keeps zoom and split when only the artwork URL changes with the same dimensions', () => {
    show(<ComparisonCanvas {...props({ view: 'split' })} />)
    act(() => buttonNamed('100%').click())
    press(slider(), 'ArrowRight')
    show(
      <ComparisonCanvas
        {...props({ view: 'split', artwork: { url: 'blob:next', width: 256, height: 192 } })}
      />,
    )
    expect(readout()).toBe('100%')
    expect(slider().getAttribute('aria-valuenow')).toBe('51')
  })

  it('keeps the viewport when the artwork pixel size changes, since the frame is the source', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    act(() => buttonNamed('100%').click())
    show(
      <ComparisonCanvas
        {...props({ view: 'artwork', artwork: { url: 'blob:wide', width: 300, height: 192 } })}
      />,
    )
    expect(readout()).toBe('100%')
  })

  it('keeps the viewport when the view mode changes, so zoom survives a switch', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    act(() => buttonNamed('100%').click())
    show(<ComparisonCanvas {...props({ view: 'split' })} />)
    expect(readout()).toBe('100%')
  })
})

describe('artwork without an original preview', () => {
  it('still shows the artwork, since that view needs only the artwork', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork', original: null })} />)
    const images = [...container.querySelectorAll('img')]
    expect(images.map((image) => image.getAttribute('alt'))).toEqual(['Binary artwork preview'])
  })

  it('shows nothing for the original view until the original preview exists', () => {
    show(<ComparisonCanvas {...props({ view: 'original', original: null })} />)
    expect(container.querySelectorAll('img')).toHaveLength(0)
  })
})

describe('vertical axis follows the source, not the artwork pixel height', () => {
  it('draws the artwork at the source height even when its pixels are taller', () => {
    show(
      <ComparisonCanvas
        {...props({
          view: 'split',
          source: { width: 256, height: 192 },
          original: { url: 'blob:original', width: 256, height: 192 },
          artwork: { url: 'blob:artwork', width: 252, height: 200 },
        })}
      />,
    )
    const [original, artwork] = [...container.querySelectorAll<HTMLImageElement>('img')]
    expect(Number.parseFloat(artwork?.style.height ?? '')).toBeCloseTo(
      Number.parseFloat(original?.style.height ?? ''),
      6,
    )
  })
})

describe('pointer capture, touch and arrow keys', () => {
  const originalRect = Element.prototype.getBoundingClientRect

  beforeEach(() => {
    Element.prototype.setPointerCapture = () => undefined
    Element.prototype.releasePointerCapture = () => undefined
    Element.prototype.hasPointerCapture = () => true
    Element.prototype.getBoundingClientRect = function rect(this: Element) {
      if (this.classList.contains('comparison-stage')) {
        return {
          x: 0,
          y: 0,
          left: 0,
          top: 0,
          right: 400,
          bottom: 300,
          width: 400,
          height: 300,
          toJSON: () => ({}),
        } as DOMRect
      }
      return originalRect.call(this)
    }
  })

  afterEach(() => {
    Element.prototype.getBoundingClientRect = originalRect
  })

  function pointerEvent(
    type: string,
    init: { pointerId: number; clientX: number; buttons: number },
  ): Event {
    return Object.assign(new Event(type, { bubbles: true, cancelable: true }), {
      button: 0,
      ...init,
    })
  }

  function firePointer(
    target: Element,
    type: string,
    init: { pointerId: number; clientX: number; buttons: number },
  ): void {
    act(() => {
      target.dispatchEvent(pointerEvent(type, init))
    })
  }

  it('gives the divider touch-action none, so touch drags do not scroll the page', () => {
    show(<ComparisonCanvas {...props({ view: 'split' })} />)
    expect(slider().style.touchAction).toBe('none')
  })

  it('moves the divider with a button held, and ignores moves once capture is lost', () => {
    show(<ComparisonCanvas {...props({ view: 'split' })} />)
    firePointer(slider(), 'pointerdown', { pointerId: 1, clientX: 100, buttons: 1 })
    firePointer(slider(), 'pointermove', { pointerId: 1, clientX: 100, buttons: 1 })
    expect(slider().getAttribute('aria-valuenow')).toBe('25')
    firePointer(slider(), 'lostpointercapture', { pointerId: 1, clientX: 0, buttons: 0 })
    firePointer(slider(), 'pointermove', { pointerId: 1, clientX: 300, buttons: 1 })
    expect(slider().getAttribute('aria-valuenow')).toBe('25')
  })

  it('ends a divider drag when a move arrives with no button held', () => {
    show(<ComparisonCanvas {...props({ view: 'split' })} />)
    firePointer(slider(), 'pointerdown', { pointerId: 2, clientX: 100, buttons: 1 })
    firePointer(slider(), 'pointermove', { pointerId: 2, clientX: 200, buttons: 0 })
    expect(slider().getAttribute('aria-valuenow')).toBe('50')
    firePointer(slider(), 'pointermove', { pointerId: 2, clientX: 300, buttons: 1 })
    expect(slider().getAttribute('aria-valuenow')).toBe('50')
  })

  it('does not suppress an arrow key that cannot pan, so the page keeps its own key handling', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    const event = new KeyboardEvent('keydown', {
      key: 'ArrowLeft',
      bubbles: true,
      cancelable: true,
    })
    let proceed = true
    act(() => {
      proceed = viewport().dispatchEvent(event)
    })
    expect(proceed).toBe(true)
  })

  it('suppresses an arrow key that does pan a zoomed image', () => {
    show(<ComparisonCanvas {...props({ view: 'artwork' })} />)
    for (let index = 0; index < 4; index += 1) act(() => buttonNamed('Zoom in').click())
    const outcomes: boolean[] = []
    for (const key of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown']) {
      const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })
      act(() => {
        outcomes.push(viewport().dispatchEvent(event))
      })
    }
    expect(outcomes.includes(false)).toBe(true)
  })
})

describe('hidden then shown', () => {
  function host(shown: boolean): ReactElement {
    return (
      <div hidden={!shown}>
        <ComparisonCanvas {...props({ view: 'artwork' })} visible={shown} />
      </div>
    )
  }

  function stageStyle(element: HTMLElement): Record<string, string> {
    const stage = element.querySelector<HTMLElement>('.comparison-stage')
    if (stage === null) throw new Error('no stage')
    return {
      width: stage.style.width,
      height: stage.style.height,
      transform: stage.style.transform,
    }
  }

  it('fits the real viewport once shown, matching a canvas mounted visible', () => {
    const freshHost = document.createElement('div')
    document.body.append(freshHost)
    const freshRoot = createRoot(freshHost)
    act(() => freshRoot.render(<ComparisonCanvas {...props({ view: 'artwork' })} />))
    const expected = stageStyle(freshHost)
    act(() => freshRoot.unmount())
    freshHost.remove()

    show(host(false))
    show(host(true))
    expect(stageStyle(container)).toEqual(expected)
    expect(readout().startsWith('Fit, ')).toBe(true)
    expect(Number.parseFloat(expected.width ?? '')).toBeGreaterThan(0)
  })
})
