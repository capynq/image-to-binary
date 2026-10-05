import { describe, expect, it, vi } from 'vitest'
import { LatestJobScheduler } from './scheduler'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

async function expectRejected(promise: Promise<unknown>) {
  await expect(promise).rejects.toBeInstanceOf(Error)
}

describe('LatestJobScheduler', () => {
  it('runs one active job and starts only the newest pending job after unwind', async () => {
    const active = deferred<string>()
    const started: number[] = []
    const scheduler = new LatestJobScheduler<string>({ releaseStaleResult: vi.fn() })
    const first = scheduler.submit(() => {
      started.push(1)
      return active.promise
    })
    const firstRejected = expectRejected(first.promise)
    await Promise.resolve()
    const second = scheduler.submit(() => {
      started.push(2)
      return 'second'
    })
    const third = scheduler.submit(() => {
      started.push(3)
      return 'third'
    })
    await expectRejected(second.promise)
    expect(started).toEqual([1])
    expect(first.generation).toBeLessThan(second.generation)
    expect(second.generation).toBeLessThan(third.generation)
    active.resolve('first stale')
    await firstRejected
    await expect(third.promise).resolves.toBe('third')
    expect(started).toEqual([1, 3])
    scheduler.dispose()
  })

  it('rejects active cancellation promptly and does not overlap the active executor', async () => {
    const active = deferred<number>()
    let running = 0
    let maxRunning = 0
    const scheduler = new LatestJobScheduler<number>({ releaseStaleResult: vi.fn() })
    const first = scheduler.submit(() => {
      running++
      maxRunning = Math.max(maxRunning, running)
      return active.promise.finally(() => running--)
    })
    await Promise.resolve()
    const second = scheduler.submit(() => {
      running++
      maxRunning = Math.max(maxRunning, running)
      running--
      return 2
    })
    await expectRejected(first.promise)
    expect(maxRunning).toBe(1)
    active.resolve(1)
    await expect(second.promise).resolves.toBe(2)
    expect(maxRunning).toBe(1)
    scheduler.dispose()
  })

  it('releases stale results once and advances after executor or cleanup failures', async () => {
    const active = deferred<object>()
    const release = vi.fn(() => {
      throw new Error('cleanup failed')
    })
    const scheduler = new LatestJobScheduler<object>({ releaseStaleResult: release })
    const first = scheduler.submit(() => active.promise)
    const firstRejected = expectRejected(first.promise)
    await Promise.resolve()
    const second = scheduler.submit(() => Promise.reject(new Error('executor failed')))
    const third = scheduler.submit(() => ({}))
    await firstRejected
    await expectRejected(second.promise)
    active.resolve({ stale: true })
    await expectRejected(first.promise)
    await expect(third.promise).resolves.toEqual({})
    expect(release).toHaveBeenCalledTimes(1)
    scheduler.dispose()
  })

  it('starts the newest pending job after the active executor rejects', async () => {
    const active = deferred<number>()
    const scheduler = new LatestJobScheduler<number>({ releaseStaleResult: vi.fn() })
    const first = scheduler.submit(() => active.promise)
    const firstRejected = expectRejected(first.promise)
    await Promise.resolve()
    const second = scheduler.submit(() => 2)
    await firstRejected
    active.reject(new Error('active executor failed'))
    await expect(second.promise).resolves.toBe(2)
    scheduler.dispose()
  })

  it('normalizes thrown values whose string conversion also throws', async () => {
    const scheduler = new LatestJobScheduler<number>({ releaseStaleResult: vi.fn() })
    const job = scheduler.submit(() => {
      throw {
        toString: () => {
          throw new Error('conversion failed')
        },
      }
    })
    await expect(job.promise).rejects.toThrow('Processing job failed.')
    scheduler.dispose()
  })

  it('cancels pending jobs and handles disposal while an executor is running', async () => {
    const active = deferred<number>()
    const executePending = vi.fn(() => 2)
    const release = vi.fn()
    const scheduler = new LatestJobScheduler<number>({ releaseStaleResult: release })
    const first = scheduler.submit(() => active.promise)
    await Promise.resolve()
    const pending = scheduler.submit(executePending)
    pending.cancel()
    await expectRejected(pending.promise)
    const last = scheduler.submit(executePending)
    const firstRejected = expectRejected(first.promise)
    scheduler.dispose()
    scheduler.dispose()
    await firstRejected
    await expectRejected(last.promise)
    active.resolve(1)
    await Promise.resolve()
    await Promise.resolve()
    expect(executePending).not.toHaveBeenCalled()
    expect(release).toHaveBeenCalledTimes(1)
    expect(() => scheduler.submit(executePending)).toThrow('Scheduler is disposed.')
  })

  it('provides cooperative cancellation checkpoints and prevents context revival', async () => {
    const active = deferred<string>()
    let context: Parameters<Parameters<LatestJobScheduler<string>['submit']>[0]>[0] | undefined
    const scheduler = new LatestJobScheduler<string>({ releaseStaleResult: vi.fn() })
    const first = scheduler.submit((jobContext) => {
      context = jobContext
      return active.promise
    })
    const firstRejected = expectRejected(first.promise)
    await Promise.resolve()
    const next = scheduler.submit(() => 'next')
    await firstRejected
    expect(context?.isCancelled()).toBe(true)
    expect(() => context?.checkpoint()).toThrow('Job was cancelled.')
    active.resolve('stale')
    await expect(next.promise).resolves.toBe('next')
    scheduler.dispose()
  })

  it('does not invoke work cancelled before its first scheduled turn', async () => {
    const execute = vi.fn(() => 1)
    const scheduler = new LatestJobScheduler<number>({ releaseStaleResult: vi.fn() })
    const job = scheduler.submit(execute)
    const rejected = expectRejected(job.promise)
    job.cancel()
    await rejected
    await Promise.resolve()
    expect(execute).not.toHaveBeenCalled()
    scheduler.dispose()

    const executeDisposed = vi.fn(() => 2)
    const disposedScheduler = new LatestJobScheduler<number>({ releaseStaleResult: vi.fn() })
    const disposedJob = disposedScheduler.submit(executeDisposed)
    const disposedRejection = expectRejected(disposedJob.promise)
    disposedScheduler.dispose()
    await disposedRejection
    await Promise.resolve()
    expect(executeDisposed).not.toHaveBeenCalled()
  })
})
