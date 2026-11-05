import { afterEach, describe, expect, it, vi } from 'vitest'
import { TASK_YIELD_TIMER_INTERVAL, createTaskYielder } from './taskYield'

describe('task yielder', () => {
  it('yields repeatedly to the task queue', async () => {
    const yielder = createTaskYielder()
    let timerRan = false
    setTimeout(() => {
      timerRan = true
    }, 0)
    for (let index = 0; index < 50 && !timerRan; index += 1) {
      await yielder.yield()
      await new Promise<void>((resolve) => setTimeout(resolve, 1))
    }
    expect(timerRan).toBe(true)
    yielder.close()
  })

  it('resolves waiters on close and stays closed', async () => {
    const yielder = createTaskYielder()
    const waiting = yielder.yield()
    yielder.close()
    await waiting
    await yielder.yield()
    yielder.close()
  })

  it('resolves every pending message-channel waiter on an idempotent close', async () => {
    const yielder = createTaskYielder()
    const waiting = [yielder.yield(), yielder.yield(), yielder.yield()]
    yielder.close()
    yielder.close()
    await Promise.all(waiting)
    await yielder.yield()
  })
})

describe('task yielder periodic timer fairness', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('uses message-channel turns until the interval, then a timer task', async () => {
    vi.useFakeTimers()
    const yielder = createTaskYielder()
    for (let index = 1; index < TASK_YIELD_TIMER_INTERVAL; index += 1) {
      await yielder.yield()
      expect(vi.getTimerCount()).toBe(0)
    }
    const boundary = yielder.yield()
    expect(vi.getTimerCount()).toBe(1)
    vi.runAllTimers()
    await boundary
    expect(vi.getTimerCount()).toBe(0)
    await yielder.yield()
    expect(vi.getTimerCount()).toBe(0)
    yielder.close()
  })

  it('lets a timer scheduled during the fast turns run before the interval boundary resolves', async () => {
    const yielder = createTaskYielder()
    let timerRan = false
    setTimeout(() => {
      timerRan = true
    }, 0)
    let yields = 0
    while (!timerRan && yields < 10 * TASK_YIELD_TIMER_INTERVAL) {
      await yielder.yield()
      yields += 1
    }
    expect(timerRan).toBe(true)
    expect(yields).toBeLessThanOrEqual(TASK_YIELD_TIMER_INTERVAL)
    yielder.close()
  })

  it('close resolves pending port and timer waiters and clears the timer', async () => {
    vi.useFakeTimers()
    const yielder = createTaskYielder()
    const waiting: Array<Promise<void>> = []
    for (let index = 0; index < TASK_YIELD_TIMER_INTERVAL; index += 1) waiting.push(yielder.yield())
    expect(vi.getTimerCount()).toBe(1)
    yielder.close()
    expect(vi.getTimerCount()).toBe(0)
    await Promise.all(waiting)
    await yielder.yield()
  })
})

describe('task yielder without MessageChannel', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('settles a fallback waiter from its timer and forgets the timer', async () => {
    vi.stubGlobal('MessageChannel', undefined)
    vi.useFakeTimers()
    const yielder = createTaskYielder()
    const waiting = yielder.yield()
    expect(vi.getTimerCount()).toBe(1)
    vi.runAllTimers()
    await waiting
    expect(vi.getTimerCount()).toBe(0)
    yielder.close()
  })

  it('close resolves unfired fallback waiters, clears timers and stays closed', async () => {
    vi.stubGlobal('MessageChannel', undefined)
    vi.useFakeTimers()
    const yielder = createTaskYielder()
    const first = yielder.yield()
    const second = yielder.yield()
    expect(vi.getTimerCount()).toBe(2)
    yielder.close()
    yielder.close()
    expect(vi.getTimerCount()).toBe(0)
    await Promise.all([first, second])
    await yielder.yield()
    expect(vi.getTimerCount()).toBe(0)
  })
})
