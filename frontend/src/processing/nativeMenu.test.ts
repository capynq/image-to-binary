import { describe, expect, it, vi } from 'vitest'
import type { NativeBridge } from './nativeBridge'
import { createNativeMenu, readMenuAction, NATIVE_MENU_EVENT, syncMenuState } from './nativeMenu'
import type { MenuState, NativeMenu } from './nativeMenu'

interface Call {
  readonly command: string
  readonly args?: Record<string, unknown> | undefined
}

const STATE: MenuState = {
  export: true,
  render: false,
  original: true,
  artwork: false,
  cancel: true,
}

function bridge(calls: Call[], reject?: string) {
  const listeners = new Map<string, (payload: unknown) => void>()
  const fake: NativeBridge = {
    invoke: <T>(command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args })
      if (command === reject) return Promise.reject(new Error('refused'))
      return Promise.resolve(undefined as T)
    },
    channel: () => {
      throw new Error('not used')
    },
    listen: (event, handler) => {
      listeners.set(event, handler as (payload: unknown) => void)
      return Promise.resolve(() => listeners.delete(event))
    },
  }
  return { fake, listeners }
}

describe('readMenuAction', () => {
  it('accepts only the known studio actions', () => {
    expect(readMenuAction({ action: 'export' })).toBe('export')
    expect(readMenuAction({ action: 'cancel' })).toBe('cancel')
    expect(readMenuAction({ action: 'quit' })).toBeNull()
    expect(readMenuAction({ action: 7 })).toBeNull()
    expect(readMenuAction(null)).toBeNull()
    expect(readMenuAction('export')).toBeNull()
  })
})

describe('createNativeMenu', () => {
  it('sends exactly the five enabled flags to the shell', async () => {
    const calls: Call[] = []
    const { fake } = bridge(calls)
    await createNativeMenu(fake).setState(STATE)
    expect(calls).toEqual([{ command: 'set_menu_state', args: { state: STATE } }])
  })

  it('rejects when the shell refuses the state, so the caller can keep the previous state', async () => {
    const calls: Call[] = []
    const { fake } = bridge(calls, 'set_menu_state')
    await expect(createNativeMenu(fake).setState(STATE)).rejects.toThrow('refused')
  })

  it('delivers known actions and drops malformed payloads', async () => {
    const calls: Call[] = []
    const { fake, listeners } = bridge(calls)
    const received: string[] = []
    await createNativeMenu(fake).listen((action) => received.push(action))
    const handler = listeners.get(NATIVE_MENU_EVENT)
    handler?.({ action: 'render' })
    handler?.({ action: 'unknown' })
    handler?.(null)
    expect(received).toEqual(['render'])
  })
})

function refusingMenu(calls: { count: number }, accept = false): NativeMenu {
  return {
    setState: () => {
      calls.count += 1
      return accept ? Promise.resolve() : Promise.reject(new Error('refused'))
    },
    listen: () => Promise.resolve(() => undefined),
  }
}

describe('syncMenuState', () => {
  it('retries a refused state once on the next tick, then reports a second refusal', async () => {
    vi.useFakeTimers()
    try {
      const calls = { count: 0 }
      const report = vi.fn()
      syncMenuState(refusingMenu(calls), STATE, report)
      await vi.advanceTimersByTimeAsync(10)
      expect(calls.count).toBe(2)
      expect(report).toHaveBeenCalledTimes(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('sends an accepted state once and reports nothing', async () => {
    vi.useFakeTimers()
    try {
      const calls = { count: 0 }
      const report = vi.fn()
      syncMenuState(refusingMenu(calls, true), STATE, report)
      await vi.advanceTimersByTimeAsync(10)
      expect(calls.count).toBe(1)
      expect(report).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })

  it('cancels a pending retry, so a superseded state is never re-sent', async () => {
    vi.useFakeTimers()
    try {
      const calls = { count: 0 }
      const cancel = syncMenuState(refusingMenu(calls), STATE, vi.fn())
      await Promise.resolve()
      await Promise.resolve()
      cancel()
      await vi.advanceTimersByTimeAsync(10)
      expect(calls.count).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})
