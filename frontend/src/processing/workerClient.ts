import {
  MAX_RETAINED_SOURCES,
  eventData,
  isArrayBuffer,
  isWorkerReply,
  validateEncodedImport,
  validateFrameIndex,
  validateImageMetadata,
  validatePreviewFrame,
  validateRgbaImage,
} from './protocol'
import type {
  ExportChunkReply,
  ExportProgressReply,
  ExportRequest,
  ExportedReply,
  ImportEncodedRequest,
  ImportedReply,
  ImportRequest,
  OriginalPreviewRequest,
  OutputKindName,
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

/**
 * Owned worker-side source. `dispose()` is idempotent and releases the Rust session. Stills report
 * `frameCount` 1, no delays and loop count 0.
 */
export interface SourceHandle extends Releasable {
  readonly sourceId: number
  readonly name: string
  readonly width: number
  readonly height: number
  readonly frameCount: number
  readonly frameDelays: ReadonlyArray<number>
  readonly loopCount: number
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

/** Encoded BMP, GIF, JPEG, PNG, TIFF or WebP bytes. Importing transfers (detaches) `bytes`. */
export interface EncodedImport {
  readonly name: string
  readonly bytes: ArrayBuffer
}

/** A producer runs inside the scheduled operation and is skipped if the operation is superseded first. */
export type EncodedImportSource = EncodedImport | (() => EncodedImport | Promise<EncodedImport>)

/** Preview pixels owned by the receiver. */
export interface PreviewPixels {
  readonly width: number
  readonly height: number
  readonly pixels: ArrayBuffer
}

/**
 * Adapter-side consumer of one export. Both methods run on the client's message path and
 * a throw fails the export and cancels the worker job; no later chunk reaches the sink.
 *
 * Chunks are byte-level and may split a multi-byte UTF-8 character, so a sink must keep the raw
 * bytes and never decode a chunk on its own.
 *
 * Backpressure: the worker posts chunks without acknowledgement, so chunks can queue on the
 * port until the 256 MiB encoded ceiling. The sink must therefore keep every accepted chunk
 * (the adapter's accumulator does) and bound its own work per call. Memory is bounded by that
 * ceiling but is not constant, and the finished Blob may transiently hold about twice the
 * encoded size while its parts are copied.
 */
export interface ExportSink {
  /** Takes ownership of one ordered, transferred chunk. */
  accept(chunk: ArrayBuffer): void
  /** Receives a fraction in [0, 1] for the current export only. */
  progress(fraction: number): void
}

/** Worker totals, already checked against the chunks the sink accepted. */
export interface ExportCompletion {
  readonly chunkCount: number
  readonly byteLength: number
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
  /** Decoded dimensions come from the worker reply; the returned name must equal the input name. */
  importEncoded(source: EncodedImportSource, operationId: number): Promise<SourceHandle>
  /**
   * `finalize` runs inside the scheduled operation; its result is released if superseded. `frame`
   * must be an index below the source's `frameCount`; a reject is a `RangeError`.
   */
  preview<T extends Releasable>(
    source: SourceHandle,
    parameters: PreviewParameters,
    operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
    frame?: number,
  ): Promise<T>
  previewPixels(
    source: SourceHandle,
    parameters: PreviewParameters,
    operationId: number,
  ): Promise<PreviewPixels>
  /** Bounded original-image preview of a retained source; `finalize` and `frame` follow the `preview` contract. */
  originalPreview<T extends Releasable>(
    source: SourceHandle,
    operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
    frame?: number,
  ): Promise<T>
  originalPreviewPixels(source: SourceHandle, operationId: number): Promise<PreviewPixels>
  /**
   * Streams an export of `source` in `outputKind` through the shared scheduler. `finalize` runs
   * only after the export completed and remained current, so its result is never published when
   * superseded.
   */
  exportFile<T extends Releasable>(
    source: SourceHandle,
    parameters: PreviewParameters,
    outputKind: OutputKindName,
    operationId: number,
    sink: ExportSink,
    finalize: (completion: ExportCompletion) => T,
    frame?: number,
  ): Promise<T>
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

type RequestKind = 'import' | 'preview' | 'export'

type RequestReply = ImportedReply | PreviewedReply | ExportedReply

type RequestMessage =
  | ImportRequest
  | ImportEncodedRequest
  | PreviewRequest
  | OriginalPreviewRequest
  | ExportRequest

const REPLY_TYPE = {
  import: 'imported',
  preview: 'previewed',
  export: 'exported',
} as const satisfies Record<RequestKind, RequestReply['type']>

/** Client-side state of one export; chunks are accepted only while the request is not cancelled. */
interface ExportStream {
  readonly sink: ExportSink
  nextSequence: number
  receivedBytes: number
  failure: Error | null
}

interface PendingRequest {
  readonly requestId: number
  readonly operationId: number
  readonly kind: RequestKind
  cancelSent: boolean
  readonly stream: ExportStream | null
  readonly resolve: (reply: RequestReply) => void
  readonly reject: (reason: Error) => void
}

interface PreparedImport {
  readonly request: ImportRequest | ImportEncodedRequest
  readonly buffer: ArrayBuffer
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
    return this.scheduleImport<RgbaImport>(operationId, source, (image, generation) => {
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
      return {
        request: {
          type: 'import',
          requestId: generation,
          operationId,
          name: image.name,
          width: image.width,
          height: image.height,
          pixels: image.pixels,
        },
        buffer: image.pixels,
      }
    })
  }

  importEncoded(source: EncodedImportSource, operationId: number): Promise<SourceHandle> {
    return this.scheduleImport<EncodedImport>(operationId, source, (image, generation) => {
      if (typeof image.name !== 'string') {
        throw new TypeError('Encoded image name must be a string.')
      }
      if (!isArrayBuffer(image.bytes)) {
        throw new TypeError('Encoded image bytes must be an ArrayBuffer.')
      }
      const invalid = validateEncodedImport(image.name, image.bytes.byteLength)
      if (invalid !== null) throw new RangeError(invalid.message)
      return {
        request: {
          type: 'importEncoded',
          requestId: generation,
          operationId,
          name: image.name,
          bytes: image.bytes,
        },
        buffer: image.bytes,
      }
    })
  }

  preview<T extends Releasable>(
    source: SourceHandle,
    parameters: PreviewParameters,
    operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
    frame = 0,
  ): Promise<T> {
    return this.schedulePreview<T>(
      source,
      operationId,
      frame,
      (generation): PreviewRequest => ({
        type: 'preview',
        requestId: generation,
        operationId,
        sourceId: source.sourceId,
        parameters: { ...parameters },
        frame,
      }),
      finalize,
    )
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

  originalPreview<T extends Releasable>(
    source: SourceHandle,
    operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
    frame = 0,
  ): Promise<T> {
    return this.schedulePreview<T>(
      source,
      operationId,
      frame,
      (generation): OriginalPreviewRequest => ({
        type: 'originalPreview',
        requestId: generation,
        operationId,
        sourceId: source.sourceId,
        frame,
      }),
      finalize,
    )
  }

  originalPreviewPixels(source: SourceHandle, operationId: number): Promise<PreviewPixels> {
    return this.originalPreview(source, operationId, (pixels) =>
      Promise.resolve({ ...pixels, dispose: () => undefined }),
    )
  }

  exportFile<T extends Releasable>(
    source: SourceHandle,
    parameters: PreviewParameters,
    outputKind: OutputKindName,
    operationId: number,
    sink: ExportSink,
    finalize: (completion: ExportCompletion) => T,
    frame?: number,
  ): Promise<T> {
    const terminal = this.terminalError()
    if (terminal !== null) return Promise.reject(terminal)
    if (this.sources.get(source.sourceId) !== source) {
      return Promise.reject(this.unownedSource())
    }
    const isGif = outputKind === 'gif'
    if (isGif && frame !== undefined && frame !== 0) {
      return Promise.reject(
        new RangeError('A GIF export covers every frame; frame must be absent.'),
      )
    }
    const selected = frame ?? 0
    const frameProblem = isGif ? null : validateFrameIndex(selected, source.frameCount)
    if (frameProblem !== null) return Promise.reject(new RangeError(frameProblem))
    return this.schedule<T>(operationId, async (context) => {
      context.checkpoint()
      this.assertUsable()
      if (this.sources.get(source.sourceId) !== source) throw this.unownedSource()
      const request: ExportRequest = {
        type: 'export',
        requestId: context.generation,
        operationId,
        sourceId: source.sourceId,
        parameters: { ...parameters },
        outputKind,
        ...(isGif ? {} : { frame: selected }),
      }
      const stream: ExportStream = { sink, nextSequence: 0, receivedBytes: 0, failure: null }
      const reply = await this.request('export', request, [], stream)
      if (reply.type !== 'exported') throw this.protocolViolation('Unexpected export reply.')
      context.checkpoint()
      return finalize({ chunkCount: reply.chunkCount, byteLength: reply.byteLength })
    })
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

  /**
   * Shared import path. The producer (if any) runs inside the scheduled job, then a
   * checkpoint and the source cap are enforced before the endpoint is opened or any
   * buffer is transferred. `prepare` validates and builds the request.
   */
  private scheduleImport<I extends { readonly name: string }>(
    operationId: number,
    source: I | (() => I | Promise<I>),
    prepare: (image: I, generation: number) => PreparedImport,
  ): Promise<SourceHandle> {
    let input: I | (() => I | Promise<I>) | null = source
    const result = this.schedule<SourceHandle>(operationId, async (context) => {
      const supplied = input
      input = null
      if (supplied === null) throw new Error('The import input was released.')
      const image =
        typeof supplied === 'function'
          ? await (supplied as () => I | Promise<I>)()
          : (supplied as I)
      context.checkpoint()
      this.assertUsable()
      const prepared = prepare(image, context.generation)
      if (this.sources.size >= this.maxSources) {
        throw new RangeError(`At most ${this.maxSources} sources can be retained at once.`)
      }
      const request = prepared.request
      const reply = await this.request('import', request, [prepared.buffer])
      if (reply.type !== 'imported') throw this.protocolViolation('Unexpected import reply.')
      if (
        reply.name !== request.name ||
        (request.type === 'import' &&
          (reply.width !== request.width || reply.height !== request.height))
      ) {
        throw this.protocolViolation('The worker returned metadata that differs from the request.')
      }
      if (validateImageMetadata(reply.name, reply.width, reply.height) !== null) {
        throw this.protocolViolation('The worker returned invalid source metadata.')
      }
      return this.registerSource(reply)
    })
    result.catch(() => {
      input = null
    })
    return result
  }

  /** Shared preview path: source ownership is checked before scheduling and again inside the job. */
  private schedulePreview<T extends Releasable>(
    source: SourceHandle,
    operationId: number,
    frame: number,
    build: (generation: number) => PreviewRequest | OriginalPreviewRequest,
    finalize: (pixels: PreviewPixels) => Promise<T>,
  ): Promise<T> {
    const terminal = this.terminalError()
    if (terminal !== null) return Promise.reject(terminal)
    if (this.sources.get(source.sourceId) !== source) {
      return Promise.reject(this.unownedSource())
    }
    const frameProblem = validateFrameIndex(frame, source.frameCount)
    if (frameProblem !== null) return Promise.reject(new RangeError(frameProblem))
    return this.schedule<T>(operationId, async (context) => {
      context.checkpoint()
      this.assertUsable()
      if (this.sources.get(source.sourceId) !== source) throw this.unownedSource()
      const reply = await this.request('preview', build(context.generation), [])
      if (reply.type !== 'previewed') throw this.protocolViolation('Unexpected preview reply.')
      if (validatePreviewFrame(reply.width, reply.height, reply.pixels.byteLength) !== null) {
        throw this.protocolViolation('The worker returned an inconsistent preview frame.')
      }
      context.checkpoint()
      return await finalize({ width: reply.width, height: reply.height, pixels: reply.pixels })
    })
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
    kind: RequestKind,
    message: RequestMessage,
    transfer: Transferable[],
    stream: ExportStream | null = null,
  ): Promise<RequestReply> {
    const endpoint = this.openEndpoint()
    return new Promise((resolve, reject) => {
      const entry: PendingRequest = {
        requestId: message.requestId,
        operationId: message.operationId,
        kind,
        cancelSent: false,
        stream,
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
      frameCount: reply.frameCount,
      frameDelays: Object.freeze([...reply.frameDelays]),
      loopCount: reply.loopCount,
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
    if (data.type === 'exportChunk' || data.type === 'exportProgress') {
      this.handleExportStream(data)
      return
    }
    const entry = this.pending.get(data.requestId)
    const kindMatches =
      data.type === 'failed' ||
      data.type === 'cancelled' ||
      (entry !== undefined && data.type === REPLY_TYPE[entry.kind])
    if (entry === undefined || entry.operationId !== data.operationId || !kindMatches) {
      this.fail(new Error('The processing worker sent an uncorrelated reply.'))
      return
    }
    const stream = entry.stream
    if (
      data.type === 'exported' &&
      stream !== null &&
      stream.failure === null &&
      !entry.cancelSent &&
      (data.chunkCount !== stream.nextSequence || data.byteLength !== stream.receivedBytes)
    ) {
      this.protocolViolation('The export totals differ from the chunks received.')
      return
    }
    this.pending.delete(entry.requestId)
    if (this.active === entry) this.active = null
    switch (data.type) {
      case 'imported':
      case 'previewed':
        entry.resolve(data)
        return
      case 'exported':
        if (stream?.failure) entry.reject(stream.failure)
        else entry.resolve(data)
        return
      case 'cancelled':
        entry.reject(stream?.failure ?? new Error('The worker acknowledged cancellation.'))
        return
      case 'failed':
        entry.reject(stream?.failure ?? new ProcessingWorkerError(data.code, data.message))
    }
  }

  /**
   * Drops any export message whose request is not the current, uncancelled export, so a
   * superseded job can never accumulate. A current message must arrive in sequence.
   */
  private handleExportStream(data: ExportChunkReply | ExportProgressReply): void {
    const entry = this.pending.get(data.requestId)
    const stream = entry?.stream ?? null
    if (entry === undefined || stream === null || entry.operationId !== data.operationId) return
    if (entry.cancelSent) return
    if (data.type === 'exportProgress') {
      this.deliverToSink(entry, () =>
        stream.sink.progress(data.total === 0 ? 0 : data.completed / data.total),
      )
      return
    }
    if (data.sequence !== stream.nextSequence) {
      this.protocolViolation('The worker sent export chunks out of order.')
      return
    }
    stream.nextSequence += 1
    stream.receivedBytes += data.bytes.byteLength
    this.deliverToSink(entry, () => stream.sink.accept(data.bytes))
  }

  private deliverToSink(entry: PendingRequest, deliver: () => void): void {
    try {
      deliver()
    } catch (error) {
      const stream = entry.stream
      if (stream !== null) stream.failure ??= asError(error)
      this.requestCancel(entry)
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
