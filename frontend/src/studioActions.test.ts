import { describe, expect, it, vi } from 'vitest'
import {
  menuStateOf,
  isTextEntry,
  runStudioAction,
  shortcutAction,
  studioGates,
} from './studioActions'
import type { StudioAction, StudioHandlers } from './studioActions'

const READY = { hasSource: true, busy: false, draftBlocked: false }

function handlers(): Record<StudioAction, ReturnType<typeof vi.fn>> {
  return {
    open: vi.fn(),
    export: vi.fn(),
    original: vi.fn(),
    artwork: vi.fn(),
    split: vi.fn(),
    render: vi.fn(),
    cancel: vi.fn(),
  }
}

describe('studio gates', () => {
  it('opens the picker at any time', () => {
    expect(studioGates({ hasSource: false, busy: true, draftBlocked: true }).open).toBe(true)
  })

  it('enables render and export only with a source, an idle studio and a valid draft', () => {
    const gates = studioGates(READY)
    expect(gates.render).toBe(true)
    expect(gates.export).toBe(true)
  })

  it('blocks render, export and the artwork view while a draft is invalid', () => {
    const gates = studioGates({ ...READY, draftBlocked: true })
    expect(gates.render).toBe(false)
    expect(gates.export).toBe(false)
    expect(gates.artwork).toBe(false)
    expect(gates.original).toBe(true)
  })

  it('gates split exactly as artwork: a source and a valid draft', () => {
    expect(studioGates(READY).split).toBe(true)
    expect(studioGates({ ...READY, draftBlocked: true }).split).toBe(false)
    expect(studioGates({ ...READY, hasSource: false }).split).toBe(false)
    expect(studioGates({ ...READY, busy: true }).split).toBe(true)
  })

  it('disables every source-dependent action without a source', () => {
    const gates = studioGates({ ...READY, hasSource: false })
    expect(gates.render).toBe(false)
    expect(gates.export).toBe(false)
    expect(gates.original).toBe(false)
    expect(gates.artwork).toBe(false)
  })

  it('refuses a second export and a render while the studio is busy', () => {
    const gates = studioGates({ ...READY, busy: true })
    expect(gates.export).toBe(false)
    expect(gates.render).toBe(false)
    expect(gates.cancel).toBe(true)
  })

  it('offers cancel only while busy', () => {
    expect(studioGates(READY).cancel).toBe(false)
  })

  it('derives the menu state from the same gates', () => {
    expect(menuStateOf(studioGates({ ...READY, draftBlocked: true }))).toEqual({
      export: false,
      render: false,
      original: true,
      artwork: false,
      split: false,
      cancel: false,
    })
  })
})

describe('runStudioAction', () => {
  it('runs an allowed action exactly once', () => {
    const table = handlers()
    expect(runStudioAction('render', studioGates(READY), table as StudioHandlers)).toBe(true)
    expect(table.render).toHaveBeenCalledTimes(1)
  })

  it('never reaches the handler of a gated action, so an invalid draft cannot render or export', () => {
    const table = handlers()
    const gates = studioGates({ ...READY, draftBlocked: true })
    expect(runStudioAction('render', gates, table as StudioHandlers)).toBe(false)
    expect(runStudioAction('export', gates, table as StudioHandlers)).toBe(false)
    expect(table.render).not.toHaveBeenCalled()
    expect(table.export).not.toHaveBeenCalled()
  })

  it('refuses export without a source and a second export while one runs', () => {
    const table = handlers()
    expect(
      runStudioAction(
        'export',
        studioGates({ ...READY, hasSource: false }),
        table as StudioHandlers,
      ),
    ).toBe(false)
    expect(
      runStudioAction('export', studioGates({ ...READY, busy: true }), table as StudioHandlers),
    ).toBe(false)
    expect(table.export).not.toHaveBeenCalled()
  })

  it('cancels only while busy', () => {
    const table = handlers()
    expect(runStudioAction('cancel', studioGates(READY), table as StudioHandlers)).toBe(false)
    expect(
      runStudioAction('cancel', studioGates({ ...READY, busy: true }), table as StudioHandlers),
    ).toBe(true)
    expect(table.cancel).toHaveBeenCalledTimes(1)
  })
})

describe('isTextEntry', () => {
  it('treats text-like inputs and textareas as text entry', () => {
    for (const type of ['text', 'search', 'url', 'tel', 'email', 'password', 'number', '']) {
      expect(isTextEntry({ tagName: 'INPUT', type })).toBe(true)
    }
    expect(isTextEntry({ tagName: 'TEXTAREA' })).toBe(true)
    expect(isTextEntry({ tagName: 'DIV', isContentEditable: true })).toBe(true)
  })

  it('does not treat controls that cannot take typed characters as text entry', () => {
    for (const type of ['range', 'checkbox', 'radio', 'color', 'button', 'file']) {
      expect(isTextEntry({ tagName: 'INPUT', type })).toBe(false)
    }
    expect(isTextEntry({ tagName: 'BUTTON' })).toBe(false)
    expect(isTextEntry(null)).toBe(false)
    expect(isTextEntry('input')).toBe(false)
  })
})

describe('shortcutAction', () => {
  const plain = { key: 'Escape', altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }

  it('maps a bare Escape outside text fields to cancel', () => {
    expect(shortcutAction({ ...plain, target: { tagName: 'BUTTON' } })).toBe('cancel')
    expect(shortcutAction({ ...plain, target: null })).toBe('cancel')
  })

  it('never fires Escape inside a text field, such as the ramp', () => {
    expect(shortcutAction({ ...plain, target: { tagName: 'INPUT', type: 'text' } })).toBeNull()
    expect(shortcutAction({ ...plain, target: { tagName: 'TEXTAREA' } })).toBeNull()
  })

  it('leaves modified keys to the browser and the menu', () => {
    expect(shortcutAction({ ...plain, metaKey: true, target: null })).toBeNull()
    expect(shortcutAction({ ...plain, shiftKey: true, target: null })).toBeNull()
  })

  it('ignores every other key, so the browser keeps its own shortcuts', () => {
    for (const key of ['Enter', 'o', 'e', 'r', '1', '2']) {
      expect(shortcutAction({ ...plain, key, target: null })).toBeNull()
    }
  })
})
