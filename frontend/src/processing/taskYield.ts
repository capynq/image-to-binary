/** Every Nth yield waits on a timer task so other task sources are serviced. */
export const TASK_YIELD_TIMER_INTERVAL = 8

/** Yields to the event loop's task queue so queued messages (such as cancel) can run. */
export interface TaskYielder {
  yield(): Promise<void>
  /** Closes the channel, clears timers and resolves every waiter; later yields resolve immediately. */
  close(): void
}

/**
 * Uses a lazily opened MessageChannel, which is not subject to the nested timer
 * clamp, for most yields. Every `TASK_YIELD_TIMER_INTERVAL`th yield waits on a
 * `setTimeout` task instead, because a self-reposting MessageChannel can run many
 * turns before other task sources, such as another port, are serviced. Without
 * MessageChannel every yield uses `setTimeout`. This bounds, but does not
 * guarantee in real time, how long other queues wait.
 */
export function createTaskYielder(): TaskYielder {
  let channel: MessageChannel | null = null
  let portWaiters: Array<() => void> = []
  const timers = new Map<ReturnType<typeof setTimeout>, () => void>()
  let yields = 0
  let closed = false

  const waitOnTimer = (resolve: () => void): void => {
    const timer = setTimeout(() => {
      timers.delete(timer)
      resolve()
    }, 0)
    timers.set(timer, resolve)
  }

  return {
    yield(): Promise<void> {
      if (closed) return Promise.resolve()
      yields += 1
      return new Promise<void>((resolve) => {
        if (typeof MessageChannel === 'undefined' || yields % TASK_YIELD_TIMER_INTERVAL === 0) {
          waitOnTimer(resolve)
          return
        }
        if (channel === null) {
          channel = new MessageChannel()
          channel.port1.onmessage = () => {
            portWaiters.shift()?.()
          }
        }
        portWaiters.push(resolve)
        channel.port2.postMessage(0)
      })
    },
    close(): void {
      closed = true
      if (channel !== null) {
        channel.port1.onmessage = null
        channel.port1.close()
        channel.port2.close()
        channel = null
      }
      const pendingPort = portWaiters
      portWaiters = []
      for (const resolve of pendingPort) resolve()
      const pendingTimers = [...timers]
      timers.clear()
      for (const [timer, resolve] of pendingTimers) {
        clearTimeout(timer)
        resolve()
      }
    },
  }
}
