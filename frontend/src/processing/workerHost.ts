import type { EngineFrame, EngineSession, ProcessingEngine } from './engine'
import {
  MAX_ENCODED_BYTES,
  MAX_RETAINED_SOURCES,
  eventData,
  isWorkerRequest,
  validateEncodedImport,
  validateImageMetadata,
  validatePreviewFrame,
  validateRgbaImage,
} from './protocol'
import type {
  ExportedReply,
  ExportRequest,
  ImportedReply,
  ImportRequest,
  ImportEncodedRequest,
  OriginalPreviewRequest,
  PreviewedReply,
  PreviewRequest,
  WorkerReply,
} from './protocol'
import { createTaskYielder } from './taskYield'
import type { TaskYielder } from './taskYield'

const DEFAULT_PIXEL_BUDGET = 131_072

/** Message surface shared by a worker global scope and a MessagePort. */
export interface HostEndpoint {
  postMessage(message: unknown, transfer: Transferable[]): void
  addEventListener(type: string, listener: (event: Event) => void): void
  removeEventListener(type: string, listener: (event: Event) => void): void
}

export interface WorkerHostOptions {
  /** Work units per `advance` call; the host yields to the task queue between calls. */
  readonly pixelBudget?: number
  /** Creates the per-job task yielder; defaults to `createTaskYielder`. */
  readonly createYielder?: () => TaskYielder
}

export interface WorkerHost {
  /** Frees every session and stops listening. Idempotent. */
  dispose(): void
}

interface SourceRecord {
  readonly session: EngineSession
}

interface ActiveJob {
  readonly requestId: number
  readonly operationId: number
  readonly sourceId: number | null
  cancelled: boolean
}

type JobRequest =
  | ImportRequest
  | ImportEncodedRequest
  | PreviewRequest
  | OriginalPreviewRequest
  | ExportRequest

type Outcome =
  | { readonly kind: 'reply'; readonly reply: WorkerReply; readonly transfer: Transferable[] }
  | { readonly kind: 'fatal'; readonly message: string }

class JobCancelled extends Error {
  constructor() {
    super('The job was cancelled.')
    this.name = 'JobCancelled'
  }
}

class EngineLoadError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'EngineLoadError'
  }
}

class JobFailure extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message)
    this.name = 'JobFailure'
  }
}

/**
 * Runs the worker side of the protocol: one job at a time, bounded `advance`
 * calls separated by task-queue yields so cancel and release messages are
 * observed, and no frame is published once a job is cancelled.
 *
 * The host owns every `ImageSession` and `PreviewFrame` wrapper it creates.
 */
export function createWorkerHost(
  endpoint: HostEndpoint,
  loadEngine: () => Promise<ProcessingEngine>,
  options: WorkerHostOptions = {},
): WorkerHost {
  const pixelBudget = options.pixelBudget ?? DEFAULT_PIXEL_BUDGET
  const createYielder = options.createYielder ?? createTaskYielder
  const sources = new Map<number, SourceRecord>()
  const doomed = new Set<SourceRecord>()
  let active: ActiveJob | null = null
  let enginePromise: Promise<ProcessingEngine> | null = null
  let nextSourceId = 1
  let lastAcceptedRequestId = 0
  let disposed = false

  const getEngine = async (): Promise<ProcessingEngine> => {
    enginePromise ??= loadEngine()
    try {
      return await enginePromise
    } catch (error) {
      throw new EngineLoadError(`The processing engine failed to load: ${errorMessage(error)}`)
    }
  }

  const assertRunning = (job: ActiveJob): void => {
    if (job.cancelled || disposed) throw new JobCancelled()
  }

  const importSource = async (
    request: ImportRequest,
    job: ActiveJob,
    engine: ProcessingEngine,
  ): Promise<ImportedReply> => {
    const invalid = validateRgbaImage(
      request.name,
      request.width,
      request.height,
      request.pixels.byteLength,
    )
    if (invalid !== null) throw new JobFailure(invalid.code, invalid.message)
    if (sources.size >= MAX_RETAINED_SOURCES) {
      throw new JobFailure('resource_limit_exceeded', 'The retained source limit was reached.')
    }
    assertRunning(job)
    const session = engine.createSession(
      request.width,
      request.height,
      new Uint8Array(request.pixels),
    )
    if (job.cancelled || disposed) {
      closeSession(session)
      throw new JobCancelled()
    }
    const sourceId = nextSourceId++
    sources.set(sourceId, { session })
    return {
      type: 'imported',
      requestId: request.requestId,
      operationId: request.operationId,
      sourceId,
      name: request.name,
      width: request.width,
      height: request.height,
    }
  }

  const importEncodedSource = async (
    request: ImportEncodedRequest,
    job: ActiveJob,
    yielder: TaskYielder,
    engine: ProcessingEngine,
  ): Promise<ImportedReply> => {
    const invalid = validateEncodedImport(request.name, request.bytes.byteLength)
    if (invalid !== null) throw new JobFailure(invalid.code, invalid.message)
    if (sources.size >= MAX_RETAINED_SOURCES) {
      throw new JobFailure('resource_limit_exceeded', 'The retained source limit was reached.')
    }
    assertRunning(job)
    const session = engine.createEncodedSession(new Uint8Array(request.bytes))
    let published = false
    try {
      const { width, height } = readNormalizedMetadata(session, request.name)
      await yielder.yield()
      assertRunning(job)
      const sourceId = nextSourceId++
      sources.set(sourceId, { session })
      published = true
      return {
        type: 'imported',
        requestId: request.requestId,
        operationId: request.operationId,
        sourceId,
        name: request.name,
        width,
        height,
      }
    } finally {
      if (!published) closeSession(session)
    }
  }

  const previewSource = async (
    request: PreviewRequest | OriginalPreviewRequest,
    job: ActiveJob,
    yielder: TaskYielder,
    engine: ProcessingEngine,
  ): Promise<PreviewedReply> => {
    const record = sources.get(request.sourceId)
    if (record === undefined) {
      throw new JobFailure('unknown_source', 'The source is not available in the worker.')
    }
    const generation = request.requestId
    let started = false
    let finished = false
    try {
      assertRunning(job)
      let stage: number
      if (request.type === 'preview') {
        const { cellSize, fontSize, threshold, sampling, mode, ramp } = request.parameters
        const { foreground, background, colorMode, contrast, invert } = request.parameters
        stage = record.session.startPreview(
          generation,
          cellSize,
          fontSize,
          threshold,
          sampling,
          mode,
          ramp,
          foreground,
          background,
          colorMode,
          contrast,
          invert,
        )
      } else {
        stage = record.session.startOriginalPreview(generation)
      }
      started = true
      while (stage !== engine.readyStage) {
        if (stage === engine.failedStage) {
          throw new JobFailure('internal', 'The preview job failed.')
        }
        assertRunning(job)
        await yielder.yield()
        assertRunning(job)
        stage = record.session.advance(generation, pixelBudget)
      }
      assertRunning(job)
      const frame = record.session.finish(generation)
      finished = true
      const taken = takeFrame(frame)
      assertRunning(job)
      if (taken.buffer.byteLength !== taken.width * taken.height * 4) {
        throw new JobFailure('internal', 'The preview frame size is inconsistent.')
      }
      if (request.type === 'originalPreview') {
        const bounded = validatePreviewFrame(taken.width, taken.height, taken.buffer.byteLength)
        if (bounded !== null) throw new JobFailure('internal', bounded.message)
      }
      return {
        type: 'previewed',
        requestId: request.requestId,
        operationId: request.operationId,
        width: taken.width,
        height: taken.height,
        pixels: taken.buffer,
      }
    } catch (error) {
      if (started && !finished && !disposed && !doomed.has(record)) {
        try {
          record.session.cancel(generation)
        } catch {}
      }
      throw error
    }
  }

  /**
   * Streams the PNG as ordered `exportChunk` messages, each transferring its own buffer,
   * then returns the `exported` totals. Every advance is followed by a drain and a progress
   * message, and yields first so cancel and release messages are observed between advances.
   */
  const exportSource = async (
    request: ExportRequest,
    job: ActiveJob,
    yielder: TaskYielder,
    engine: ProcessingEngine,
  ): Promise<ExportedReply> => {
    const record = sources.get(request.sourceId)
    if (record === undefined) {
      throw new JobFailure('unknown_source', 'The source is not available in the worker.')
    }
    const { session } = record
    const generation = request.requestId
    const { cellSize, fontSize, threshold, sampling, mode, ramp } = request.parameters
    const { foreground, background, colorMode, contrast, invert } = request.parameters
    const emitted = { sequence: 0, byteLength: 0 }
    const postChunks = (chunks: Uint8Array[]): void => {
      for (const chunk of chunks) {
        if (chunk.byteLength === 0) continue
        assertRunning(job)
        if (emitted.byteLength + chunk.byteLength > MAX_ENCODED_BYTES) {
          throw new JobFailure('resource_limit_exceeded', 'The encoded PNG exceeds the byte limit.')
        }
        const bytes = exactBuffer(chunk)
        const length = bytes.byteLength
        endpoint.postMessage(
          {
            type: 'exportChunk',
            requestId: request.requestId,
            operationId: request.operationId,
            sequence: emitted.sequence,
            bytes,
          } satisfies WorkerReply,
          [bytes],
        )
        emitted.sequence += 1
        emitted.byteLength += length
      }
    }
    let started = false
    let finished = false
    try {
      assertRunning(job)
      let stage = session.startExport(
        generation,
        cellSize,
        fontSize,
        threshold,
        sampling,
        mode,
        ramp,
        foreground,
        background,
        colorMode,
        contrast,
        invert,
      )
      started = true
      while (stage !== engine.readyStage) {
        if (stage === engine.failedStage) {
          throw new JobFailure('internal', 'The export job failed.')
        }
        assertRunning(job)
        await yielder.yield()
        assertRunning(job)
        stage = session.advance(generation, pixelBudget)
        postChunks(session.drainExport(generation))
        const total = session.progressTotal()
        assertRunning(job)
        endpoint.postMessage(
          {
            type: 'exportProgress',
            requestId: request.requestId,
            operationId: request.operationId,
            completed: Math.min(session.progressCompleted(), total),
            total,
          } satisfies WorkerReply,
          [],
        )
      }
      assertRunning(job)
      const finalChunks = session.finishExport(generation)
      finished = true
      postChunks(finalChunks)
      return {
        type: 'exported',
        requestId: request.requestId,
        operationId: request.operationId,
        chunkCount: emitted.sequence,
        byteLength: emitted.byteLength,
      }
    } catch (error) {
      if (started && !finished && !disposed && !doomed.has(record)) {
        try {
          session.cancel(generation)
        } catch {}
      }
      throw error
    }
  }

  const execute = async (
    request: JobRequest,
    job: ActiveJob,
    yielder: TaskYielder,
  ): Promise<Outcome> => {
    try {
      assertRunning(job)
      await yielder.yield()
      assertRunning(job)
      const engine = await getEngine()
      assertRunning(job)
      if (request.type === 'import') {
        return { kind: 'reply', reply: await importSource(request, job, engine), transfer: [] }
      }
      if (request.type === 'importEncoded') {
        return {
          kind: 'reply',
          reply: await importEncodedSource(request, job, yielder, engine),
          transfer: [],
        }
      }
      if (request.type === 'export') {
        return {
          kind: 'reply',
          reply: await exportSource(request, job, yielder, engine),
          transfer: [],
        }
      }
      const reply = await previewSource(request, job, yielder, engine)
      return { kind: 'reply', reply, transfer: [reply.pixels] }
    } catch (error) {
      if (error instanceof EngineLoadError) return { kind: 'fatal', message: error.message }
      const identity = { requestId: request.requestId, operationId: request.operationId }
      if (error instanceof JobCancelled) {
        return { kind: 'reply', reply: { type: 'cancelled', ...identity }, transfer: [] }
      }
      return {
        kind: 'reply',
        reply: {
          type: 'failed',
          ...identity,
          code: errorCode(error),
          message: errorMessage(error),
        },
        transfer: [],
      }
    }
  }

  const raiseFatal = (message: string): void => {
    try {
      endpoint.postMessage({ type: 'fatal', message } satisfies WorkerReply, [])
    } catch {}
    dispose()
  }

  const run = async (request: JobRequest): Promise<void> => {
    const job: ActiveJob = {
      requestId: request.requestId,
      operationId: request.operationId,
      sourceId:
        request.type === 'preview' ||
        request.type === 'originalPreview' ||
        request.type === 'export'
          ? request.sourceId
          : null,
      cancelled: false,
    }
    active = job
    const yielder = createYielder()
    let outcome: Outcome
    try {
      outcome = await execute(request, job, yielder)
    } finally {
      yielder.close()
      active = null
      for (const record of doomed) closeSession(record.session)
      doomed.clear()
    }
    if (disposed) return
    if (outcome.kind === 'fatal') {
      raiseFatal(outcome.message)
      return
    }
    try {
      endpoint.postMessage(outcome.reply, outcome.transfer)
    } catch (error) {
      try {
        endpoint.postMessage(
          {
            type: 'failed',
            requestId: request.requestId,
            operationId: request.operationId,
            code: 'internal',
            message: errorMessage(error),
          } satisfies WorkerReply,
          [],
        )
      } catch {
        raiseFatal('The worker could not post a reply.')
      }
    }
  }

  const releaseSource = (sourceId: number): void => {
    const record = sources.get(sourceId)
    if (record === undefined) return
    sources.delete(sourceId)
    const job = active
    if (job !== null && job.sourceId === sourceId) {
      job.cancelled = true
      doomed.add(record)
    } else {
      closeSession(record.session)
    }
  }

  const onMessage = (event: Event): void => {
    if (disposed) return
    const data = eventData(event)
    if (!isWorkerRequest(data)) {
      raiseFatal('The worker received a malformed request.')
      return
    }
    switch (data.type) {
      case 'cancel': {
        const job = active
        if (
          job !== null &&
          job.requestId === data.requestId &&
          job.operationId === data.operationId
        ) {
          job.cancelled = true
        }
        return
      }
      case 'releaseSource':
        releaseSource(data.sourceId)
        return
      case 'import':
      case 'importEncoded':
      case 'preview':
      case 'originalPreview':
      case 'export':
        if (data.requestId <= lastAcceptedRequestId) {
          raiseFatal('The worker received a replayed or out-of-order request id.')
          return
        }
        if (active !== null) {
          try {
            endpoint.postMessage(
              {
                type: 'failed',
                requestId: data.requestId,
                operationId: data.operationId,
                code: 'busy',
                message: 'The worker is already running a job.',
              } satisfies WorkerReply,
              [],
            )
          } catch {}
          return
        }
        lastAcceptedRequestId = data.requestId
        void run(data)
    }
  }

  const onMessageError = (): void => {
    raiseFatal('The worker could not deserialize a message.')
  }

  function dispose(): void {
    if (disposed) return
    disposed = true
    if (active !== null) active.cancelled = true
    endpoint.removeEventListener('message', onMessage)
    endpoint.removeEventListener('messageerror', onMessageError)
    for (const record of sources.values()) closeSession(record.session)
    for (const record of doomed) closeSession(record.session)
    sources.clear()
    doomed.clear()
  }

  endpoint.addEventListener('message', onMessage)
  endpoint.addEventListener('messageerror', onMessageError)
  return { dispose }
}

function closeSession(session: EngineSession): void {
  try {
    session.dispose()
  } catch {}
  try {
    session.free()
  } catch {}
}

/** Returns a buffer spanning exactly the view, transferring the original when it already does. */
function exactBuffer(view: Uint8Array): ArrayBuffer {
  if (
    view.buffer instanceof ArrayBuffer &&
    view.byteOffset === 0 &&
    view.byteLength === view.buffer.byteLength
  ) {
    return view.buffer
  }
  const copy = new Uint8Array(view.byteLength)
  copy.set(view)
  return copy.buffer
}

function closeFrame(frame: EngineFrame): void {
  try {
    frame.free()
  } catch {}
}

function readNormalizedMetadata(
  session: EngineSession,
  name: string,
): { readonly width: number; readonly height: number } {
  const width = session.sourceWidth
  const height = session.sourceHeight
  const channels = session.sourceChannels
  if (channels !== 4 || validateImageMetadata(name, width, height) !== null) {
    throw new JobFailure('internal', 'The decoded source metadata is invalid.')
  }
  return { width, height }
}

function takeFrame(frame: EngineFrame): {
  readonly width: number
  readonly height: number
  readonly buffer: ArrayBuffer
} {
  try {
    const width = frame.width
    const height = frame.height
    const pixels = frame.takePixels()
    const buffer =
      pixels.byteOffset === 0 && pixels.byteLength === pixels.buffer.byteLength
        ? pixels.buffer
        : pixels.slice().buffer
    return { width, height, buffer }
  } finally {
    closeFrame(frame)
  }
}

function errorCode(error: unknown): string {
  if (error instanceof JobFailure) return error.code
  if (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    typeof error.code === 'string'
  ) {
    return error.code
  }
  return 'internal'
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  try {
    return String(error)
  } catch {
    return 'Unknown worker error.'
  }
}
