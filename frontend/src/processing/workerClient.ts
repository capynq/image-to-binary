import {
  MAX_RETAINED_SOURCES,
  eventData,
  isArrayBuffer,
  isWorkerReply,
  validatePreviewFrame,
  validateRgbaImage,
} from './protocol'
import type {
  ImportedReply,
  ImportRequest,
  PreviewParameters,
  PreviewRequest,
  PreviewedReply,
} from './protocol'
import { LatestJobScheduler } from './scheduler'
import type { ScheduledJob, ScheduledJobContext } from './scheduler'

/** Message surface the client needs; `Worker` satisfies it. */
export interface ClientEndpoint {
  postMessage(message: unknown, transfer: Transferable[]): void
  addEventListener(type: string, listener: (event: Event) => void): void
  removeEventListener(type: string, listener: (event: Event) => void): void
  terminate(): void
}

export interface Releasable {
  dispose(): void
}

/** Owned worker-side source. `dispose()` is idempotent and releases the Rust session. */
export interface SourceHandle extends Releasable {
  readonly sourceId: number
  readonly name: string
  readonly width: number
  readonly height: number
}

/** Normalized RGBA8 input. Importing transfers (detaches) `pixels`. */
export interface RgbaImport {
  readonly name: string
  readonly width: number
  readonly height: number
  readonly pixels: ArrayBuffer
}

/** A producer runs inside the scheduled operation, so conversion work is serialized too. */
export type RgbaImportSource = RgbaImport | (() => RgbaImport | Promise<RgbaImport>)

/** Preview pixels owned by the receiver. */
export interface PreviewPixels {
  readonly width: number
  readonly height: number
  readonly pixels: ArrayBuffer
}

export interface ProcessingClientOptions {
  /** Called lazily on the first operation; never during construction. */
  readonly createEndpoint: () => ClientEndpoint
  readonly maxSources?: number
}

/**
 * Lower-level seam over one lazily created worker. The client owns the worker,
 * the scheduler and every source handle it returns; `dispose()` terminates the
 * worker and rejects all outstanding work. A worker crash fails the session
 * deterministically and is never silently restarted.
 */
export interface ProcessingClient {
  importRgba(source: RgbaImportSource, operationId: number): Promise<SourceHandle>
  /** `finalize` runs inside the scheduled operation; its result is released if superseded. */
  preview<T extends Releasable>(
    source: SourceHandle,
    parameters: PreviewParameters,
    operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
  ): Promise<T>
  previewPixels(
    source: SourceHandle,
    parameters: PreviewParameters,
    operationId: number,
  ): Promise<PreviewPixels>
  cancel(operationId: number): void
  dispose(): void
}

export class ProcessingWorkerError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message)
    this.name = 'ProcessingWorkerError'
  }
}

interface PendingRequest {
  readonly requestId: number
  readonly operationId: number
  readonly kind: 'import' | 'preview'
  cancelSent: boolean
  readonly resolve: (reply: ImportedReply | PreviewedReply) => void
  readonly reject: (reason: Error) => void
}

export function createProcessingClient(options: ProcessingClientOptions): ProcessingClient {
  return new WorkerProcessingClient(options)
}

class WorkerProcessingClient implements ProcessingClient {
  private endpoint: ClientEndpoint | null = null
  private failure: Error | null = null
  private disposed = false
  private active: PendingRequest | null = null
  private readonly pending = new Map<number, PendingRequest>()
  private readonly jobs = new Map<number, ScheduledJob<Releasable>>()
  private readonly sources = new Map<number, SourceHandle>()
  private readonly scheduler = new LatestJobScheduler<Releasable>({
    releaseStaleResult: (result) => result.dispose(),
  })
  private readonly maxSources: number

  constructor(private readonly options: ProcessingClientOptions) {
    this.maxSources = options.maxSources ?? MAX_RETAINED_SOURCES
  }

  importRgba(source: RgbaImportSource, operationId: number): Promise<SourceHandle> {
    let input: RgbaImportSource | null = source
    const result = this.schedule<SourceHandle>(operationId, async (context) => {
      const supplied = input
      input = null
      if (supplied === null) throw new Error('The import input was released.')
      const image = typeof supplied === 'function' ? await supplied() : supplied
      context.checkpoint()
      this.assertUsable()
      if (!isArrayBuffer(image.pixels)) {
        throw new TypeError('Import pixels must be an ArrayBuffer.')
      }
      const invalid = validateRgbaImage(
        image.name,
        image.width,
        image.height,
        image.pixels.byteLength,
      )
      if (invalid !== null) throw new RangeError(invalid.message)
      if (this.sources.size >= this.maxSources) {
        throw new RangeError(`At most ${this.maxSources} sources can be retained at once.`)
      }
      const request: ImportRequest = {
        type: 'import',
        requestId: context.generation,
        operationId,
        name: image.name,
        width: image.width,
        height: image.height,
        pixels: image.pixels,
      }
      const reply = await this.request('import', request, [image.pixels])
      if (reply.type !== 'imported') throw this.protocolViolation('Unexpected import reply.')
      if (
        reply.name !== request.name ||
        reply.width !== request.width ||
        reply.height !== request.height
      ) {
        throw this.protocolViolation('The worker returned metadata that differs from the request.')
      }
      return this.registerSource(reply)
    })
    result.catch(() => {
      input = null
    })
    return result
  }

  preview<T extends Releasable>(
    source: SourceHandle,
    parameters: PreviewParameters,
    operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
  ): Promise<T> {
    const terminal = this.terminalError()
    if (terminal !== null) return Promise.reject(terminal)
    if (this.sources.get(source.sourceId) !== source) {
      return Promise.reject(this.unownedSource())
    }
    return this.schedule<T>(operationId, async (context) => {
      context.checkpoint()
      this.assertUsable()
      if (this.sources.get(source.sourceId) !== source) throw this.unownedSource()
      const request: PreviewRequest = {
        type: 'preview',
        requestId: context.generation,
        operationId,
        sourceId: source.sourceId,
        parameters: { ...parameters },
      }
      const reply = await this.request('preview', request, [])
      if (reply.type !== 'previewed') throw this.protocolViolation('Unexpected preview reply.')
      if (validatePreviewFrame(reply.width, reply.height, reply.pixels.byteLength) !== null) {
        throw this.protocolViolation('The worker returned an inconsistent preview frame.')
      }
      return await finalize({ width: reply.width, height: reply.height, pixels: reply.pixels })
    })
  }

  previewPixels(
    source: SourceHandle,
    parameters: PreviewParameters,
    operationId: number,
  ): Promise<PreviewPixels> {
    return this.preview(source, parameters, operationId, (pixels) =>
      Promise.resolve({ ...pixels, dispose: () => undefined }),
    )
  }

  cancel(operationId: number): void {
    const job = this.jobs.get(operationId)
    if (job === undefined) return
    if (this.active !== null && this.active.operationId === operationId) {
      this.requestCancel(this.active)
    }
    job.cancel()
  }

  dispose(): void {
    if (this.disposed) return
    this.disposed = true
    this.teardown(new Error('Processing client is disposed.'))
  }

  private schedule<T extends Releasable>(
    operationId: number,
    run: (context: ScheduledJobContext) => Promise<T>,
  ): Promise<T> {
    const terminal = this.terminalError()
    if (terminal !== null) return Promise.reject(terminal)
    if (this.jobs.has(operationId)) {
      return Promise.reject(new Error('Operation id is already in use.'))
    }
    if (this.active !== null) this.requestCancel(this.active)
    const box: { value: T | null } = { value: null }
    let job: ScheduledJob<Releasable>
    try {
      job = this.scheduler.submit(async (context) => {
        const value = await run(context)
        box.value = value
        return value
      })
    } catch (error) {
      return Promise.reject(asError(error))
    }
    this.jobs.set(operationId, job)
    return job.promise
      .then(
        () => {
          const value = box.value
          if (value === null) throw new Error('The operation produced no result.')
          return value
        },
        (error: unknown) => {
          throw this.terminalError() ?? asError(error)
        },
      )
      .finally(() => {
        if (this.jobs.get(operationId) === job) this.jobs.delete(operationId)
      })
  }

  private request(
    kind: 'import' | 'preview',
    message: ImportRequest | PreviewRequest,
    transfer: Transferable[],
  ): Promise<ImportedReply | PreviewedReply> {
    const endpoint = this.openEndpoint()
    return new Promise((resolve, reject) => {
      const entry: PendingRequest = {
        requestId: message.requestId,
        operationId: message.operationId,
        kind,
        cancelSent: false,
        resolve,
        reject,
      }
      this.pending.set(entry.requestId, entry)
      this.active = entry
      try {
        endpoint.postMessage(message, transfer)
      } catch (error) {
        this.pending.delete(entry.requestId)
        if (this.active === entry) this.active = null
        reject(asError(error))
      }
    })
  }

  private openEndpoint(): ClientEndpoint {
    this.assertUsable()
    if (this.endpoint !== null) return this.endpoint
    let endpoint: ClientEndpoint
    try {
      endpoint = this.options.createEndpoint()
    } catch (error) {
      const failure = asError(error)
      this.fail(failure)
      throw failure
    }
    endpoint.addEventListener('message', this.handleMessage)
    endpoint.addEventListener('messageerror', this.handleMessageError)
    endpoint.addEventListener('error', this.handleError)
    this.endpoint = endpoint
    return endpoint
  }

  private requestCancel(entry: PendingRequest): void {
    if (entry.cancelSent || this.endpoint === null) return
    entry.cancelSent = true
    try {
      this.endpoint.postMessage(
        { type: 'cancel', requestId: entry.requestId, operationId: entry.operationId },
        [],
      )
    } catch (error) {
      this.fail(asError(error))
    }
  }

  private registerSource(reply: ImportedReply): SourceHandle {
    if (this.sources.has(reply.sourceId)) {
      throw this.protocolViolation('The worker reused the id of a live source.')
    }
    const handle: SourceHandle = {
      sourceId: reply.sourceId,
      name: reply.name,
      width: reply.width,
      height: reply.height,
      dispose: () => this.releaseHandle(handle),
    }
    this.sources.set(handle.sourceId, handle)
    return handle
  }

  private releaseHandle(handle: SourceHandle): void {
    if (this.sources.get(handle.sourceId) !== handle) return
    this.sources.delete(handle.sourceId)
    if (this.endpoint === null) return
    try {
      this.endpoint.postMessage({ type: 'releaseSource', sourceId: handle.sourceId }, [])
    } catch (error) {
      this.fail(asError(error))
    }
  }

  private readonly handleMessage = (event: Event): void => {
    const data = eventData(event)
    if (!isWorkerReply(data)) {
      this.fail(new Error('The processing worker sent a malformed message.'))
      return
    }
    if (data.type === 'fatal') {
      this.fail(new ProcessingWorkerError('worker_fatal', data.message))
      return
    }
    const entry = this.pending.get(data.requestId)
    const kindMatches =
      data.type === 'failed' ||
      data.type === 'cancelled' ||
      (entry !== undefined && data.type === (entry.kind === 'import' ? 'imported' : 'previewed'))
    if (entry === undefined || entry.operationId !== data.operationId || !kindMatches) {
      this.fail(new Error('The processing worker sent an uncorrelated reply.'))
      return
    }
    this.pending.delete(entry.requestId)
    if (this.active === entry) this.active = null
    switch (data.type) {
      case 'imported':
      case 'previewed':
        entry.resolve(data)
        return
      case 'cancelled':
        entry.reject(new Error('The worker acknowledged cancellation.'))
        return
      case 'failed':
        entry.reject(new ProcessingWorkerError(data.code, data.message))
    }
  }

  private readonly handleMessageError = (): void => {
    this.fail(new Error('The processing worker sent a message that could not be deserialized.'))
  }

  private readonly handleError = (event: Event): void => {
    const detail =
      'message' in event && typeof event.message === 'string' && event.message !== ''
        ? event.message
        : 'The processing worker failed.'
    this.fail(new Error(detail))
  }

  private fail(error: Error): void {
    if (this.failure !== null || this.disposed) return
    this.failure = error
    this.teardown(error)
  }

  private teardown(reason: Error): void {
    const endpoint = this.endpoint
    this.endpoint = null
    if (endpoint !== null) {
      endpoint.removeEventListener('message', this.handleMessage)
      endpoint.removeEventListener('messageerror', this.handleMessageError)
      endpoint.removeEventListener('error', this.handleError)
      try {
        endpoint.terminate()
      } catch {}
    }
    const entries = [...this.pending.values()]
    this.pending.clear()
    this.active = null
    this.scheduler.dispose()
    this.sources.clear()
    this.jobs.clear()
    for (const entry of entries) entry.reject(reason)
  }

  private terminalError(): Error | null {
    if (this.failure !== null) return this.failure
    return this.disposed ? new Error('Processing client is disposed.') : null
  }

  private assertUsable(): void {
    const terminal = this.terminalError()
    if (terminal !== null) throw terminal
  }

  private unownedSource(): Error {
    return new Error('The source handle was released or does not belong to this client.')
  }

  private protocolViolation(message: string): Error {
    const error = new Error(message)
    this.fail(error)
    return error
  }
}

function asError(error: unknown): Error {
  if (error instanceof Error) return error
  try {
    return new Error(String(error))
  } catch {
    return new Error('Processing failed.')
  }
}
