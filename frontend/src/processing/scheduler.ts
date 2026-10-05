export interface ScheduledJobContext {
  readonly generation: number
  readonly isCancelled: () => boolean
  checkpoint(): void
}

export interface ScheduledJob<T> {
  readonly generation: number
  readonly promise: Promise<T>
  cancel(): void
}

export interface LatestJobSchedulerOptions<T> {
  /** Releases results that complete after cancellation; cleanup errors are contained. */
  readonly releaseStaleResult: (result: T) => void
}

interface Job<T> {
  readonly generation: number
  readonly execute: (context: ScheduledJobContext) => Promise<T> | T
  resolve: (result: T) => void
  reject: (reason: Error) => void
  settled: boolean
  cancelled: boolean
}

/** Serializes work to one active and one replaceable pending job.
 *
 * Generations are scheduler-local identities, not controller generations. Callers
 * own fulfilled results; cancelled job promises reject and must be handled.
 */
export class LatestJobScheduler<T> {
  private active: Job<T> | null = null
  private pending: Job<T> | null = null
  private generation = 0
  private disposed = false

  constructor(private readonly options: LatestJobSchedulerOptions<T>) {}

  submit(execute: (context: ScheduledJobContext) => Promise<T> | T): ScheduledJob<T> {
    if (this.disposed) {
      throw new Error('Scheduler is disposed.')
    }
    if (this.generation >= Number.MAX_SAFE_INTEGER) {
      throw new RangeError('Scheduler generation limit reached.')
    }

    let job!: Job<T>
    const promise = new Promise<T>((resolve, reject) => {
      job = {
        generation: ++this.generation,
        execute,
        resolve,
        reject,
        settled: false,
        cancelled: false,
      }
    })

    if (this.active === null) {
      this.start(job)
    } else {
      this.cancelActive(this.active, 'Superseded by a newer job.')
      if (this.pending !== null) {
        this.cancelPending(this.pending, 'Replaced by a newer pending job.')
      }
      this.pending = job
    }

    return { generation: job.generation, promise, cancel: () => this.cancel(job) }
  }

  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    if (this.pending !== null) this.cancelPending(this.pending, 'Scheduler disposed.')
    if (this.active !== null) this.cancelActive(this.active, 'Scheduler disposed.')
  }

  private start(job: Job<T>): void {
    this.active = job
    const context: ScheduledJobContext = {
      generation: job.generation,
      isCancelled: () => job.cancelled || this.disposed,
      checkpoint: () => {
        if (job.cancelled || this.disposed) throw new Error('Job was cancelled.')
      },
    }

    Promise.resolve()
      .then(() => {
        context.checkpoint()
        return job.execute(context)
      })
      .then(
        (result) => {
          if (job.cancelled || this.disposed) {
            try {
              this.options.releaseStaleResult(result)
            } catch {}
          } else {
            this.settle(job, result)
          }
        },
        (error: unknown) => {
          if (!job.cancelled && !this.disposed) this.reject(job, asError(error))
        },
      )
      .finally(() => {
        if (this.active === job) this.active = null
        this.startPending()
      })
  }

  private cancel(job: Job<T>): void {
    if (job.settled || job.cancelled) return
    if (this.pending === job) {
      this.cancelPending(job, 'Job cancelled.')
    } else if (this.active === job) {
      this.cancelActive(job, 'Job cancelled.')
    }
  }

  private cancelActive(job: Job<T>, message: string): void {
    job.cancelled = true
    this.reject(job, new Error(message))
  }

  private cancelPending(job: Job<T>, message: string): void {
    if (this.pending === job) this.pending = null
    job.cancelled = true
    this.reject(job, new Error(message))
  }

  private startPending(): void {
    if (this.disposed || this.active !== null || this.pending === null) return
    const next = this.pending
    this.pending = null
    if (!next.cancelled) this.start(next)
  }

  private settle(job: Job<T>, result: T): void {
    if (job.settled) return
    job.settled = true
    job.resolve(result)
  }

  private reject(job: Job<T>, reason: Error): void {
    if (job.settled) return
    job.settled = true
    job.reject(reason)
  }
}

function asError(error: unknown): Error {
  if (error instanceof Error) return error
  try {
    return new Error(String(error))
  } catch {
    return new Error('Processing job failed.')
  }
}
