/** @vitest-environment jsdom */
import { afterEach, describe, expect, it } from 'vitest'
import { focusReachable, isReachable } from './studioActions'

const nodes: HTMLElement[] = []

function element(tag: string, attributes: Record<string, string> = {}): HTMLElement {
  const created = document.createElement(tag)
  for (const [name, value] of Object.entries(attributes)) created.setAttribute(name, value)
  document.body.append(created)
  nodes.push(created)
  return created
}

afterEach(() => {
  for (const node of nodes.splice(0)) node.remove()
})

describe('focusReachable', () => {
  it('focuses an enabled, visible target', () => {
    const target = element('button')
    expect(focusReachable(target, element('h1', { tabindex: '-1' }))).toBe(target)
    expect(document.activeElement).toBe(target)
  })

  it('falls back when the target is disabled', () => {
    const target = element('button', { disabled: '' })
    const fallback = element('h1', { tabindex: '-1' })
    expect(focusReachable(target, fallback)).toBe(fallback)
    expect(document.activeElement).toBe(fallback)
  })

  it('falls back when the target sits inside a hidden subtree', () => {
    const hidden = element('div', { hidden: '' })
    const target = document.createElement('button')
    hidden.append(target)
    const fallback = element('h1', { tabindex: '-1' })
    expect(isReachable(target)).toBe(false)
    expect(focusReachable(target, fallback)).toBe(fallback)
  })

  it('falls back when the target has been detached', () => {
    const target = document.createElement('button')
    const fallback = element('h1', { tabindex: '-1' })
    expect(focusReachable(target, fallback)).toBe(fallback)
  })

  it('returns null and focuses nothing when neither is reachable', () => {
    const disabled = element('button', { disabled: '' })
    expect(focusReachable(disabled, null)).toBeNull()
  })
})
