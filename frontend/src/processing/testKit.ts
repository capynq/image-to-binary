import type { EngineFrame, EngineSession, ProcessingEngine } from './engine'
import type { PreviewParameters } from './protocol'
import { createProcessingClient } from './workerClient'
import type { ClientEndpoint, ProcessingClient, RgbaImport } from './workerClient'
import { createWorkerHost } from './workerHost'
import type { HostEndpoint, WorkerHost } from './workerHost'

export const FAKE_STAGES = { sampling: 1, preview: 2, ready: 3, failed: 4, encoding: 5 } as const

export const PARAMS: PreviewParameters = {
  cellSize: 12,
  fontSize: 16,
  threshold: 128,
  sampling: 'classic',
}

export function rgba(name = 'test'): RgbaImport {
  return { name, width: 2, height: 1, pixels: new ArrayBuffer(8) }
}

export type Settled<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: Error }

/** Observes a promise immediately so its rejection is never unhandled. */
export function settle<T>(promise: Promise<T>): Promise<Settled<T>> {
  return promise.then(
    (value): Settled<T> => ({ ok: true, value }),
    (error: unknown): Settled<T> => ({
      ok: false,
      error: error instanceof Error ? error : new Error(String(error)),
    }),
  )
}

export async function until(predicate: () => boolean, label = 'condition'): Promise<void> {
  for (let attempt = 0; attempt < 500; attempt += 1) {
    if (predicate()) return
    await new Promise<void>((resolve) => setTimeout(resolve, 2))
  }
  throw new Error(`Timed out waiting for ${label}.`)
}

export class FakeFrame implements EngineFrame {
  freed = false
  taken = false

  constructor(
    readonly width: number,
    readonly height: number,
  ) {}

  takePixels(): Uint8Array<ArrayBuffer> {
    if (this.taken) throw new Error('frame consumed')
    this.taken = true
    return new Uint8Array(this.width * this.height * 4).fill(7)
  }

  free(): void {
    this.freed = true
  }
}

/** Bytes of the fake PNG chunk produced by advance `index`, as `[index, index]`. */
export function fakeChunkBytes(index: number): Uint8Array {
  return Uint8Array.of(index, index)
}

/** Concatenated bytes a complete fake export with `steps` advances must deliver. */
export function fakePngBytes(steps: number): Uint8Array {
  const parts: number[] = []
  for (let index = 1; index <= steps; index += 1) {
    if (index * 2 >= steps) parts.push(index, index)
  }
  parts.push(255)
  return Uint8Array.from(parts)
}

export class FakeSession implements EngineSession {
  disposeCalls = 0
  freed = false
  advances = 0
  readonly budgets: number[] = []
  private original = false
  private exporting = false
  private pending: Uint8Array[] = []

  constructor(
    private readonly engine: FakeEngine,
    readonly sourceWidth = 2,
    readonly sourceHeight = 1,
    readonly sourceChannels = 4,
  ) {}

  startPreview(generation: number): number {
    this.engine.log.push(`start:${generation}`)
    this.original = false
    this.exporting = false
    this.engine.onStart?.(generation)
    this.advances = 0
    return FAKE_STAGES.sampling
  }

  startExport(generation: number): number {
    this.engine.log.push(`export:${generation}`)
    this.original = false
    this.exporting = true
    this.engine.onStart?.(generation)
    this.advances = 0
    this.pending = []
    return FAKE_STAGES.sampling
  }

  drainExport(generation: number): Uint8Array[] {
    if (!this.exporting) throw new Error('no_job')
    this.engine.log.push(`drain:${generation}`)
    const drained = this.pending
    this.pending = []
    return drained
  }

  finishExport(generation: number): Uint8Array[] {
    if (!this.exporting) throw new Error('no_job')
    if (this.advances < this.engine.steps) throw new Error('job_incomplete')
    this.engine.log.push(`finishExport:${generation}`)
    const last = Uint8Array.of(255)
    this.engine.drainedBuffers.push(last.buffer)
    this.pending.push(last)
    this.exporting = false
    const drained = this.pending
    this.pending = []
    return drained
  }

  progressCompleted(): number {
    return Math.min(this.advances, this.engine.steps)
  }

  progressTotal(): number {
    return this.engine.steps
  }

  startOriginalPreview(generation: number): number {
    this.engine.log.push(`original:${generation}`)
    this.original = true
    this.engine.onStart?.(generation)
    this.advances = 0
    return FAKE_STAGES.sampling
  }

  advance(_generation: number, pixelBudget: number): number {
    this.advances += 1
    this.budgets.push(pixelBudget)
    this.engine.notifyAdvance()
    this.engine.onAdvance?.(this.advances)
    if (this.exporting) return this.advanceExport()
    if (this.advances >= this.engine.steps) return FAKE_STAGES.ready
    return this.advances * 2 >= this.engine.steps ? FAKE_STAGES.preview : FAKE_STAGES.sampling
  }

  private advanceExport(): number {
    if (this.engine.failExportAt === this.advances) {
      this.exporting = false
      throw Object.assign(new Error('The fake encoder failed.'), { code: 'encode_failed' })
    }
    const encoding = this.advances * 2 >= this.engine.steps
    if (encoding) this.pending.push(this.chunk(this.advances))
    if (this.advances >= this.engine.steps) return FAKE_STAGES.ready
    return encoding ? FAKE_STAGES.encoding : FAKE_STAGES.sampling
  }

  /** Fresh chunk; a sub-view option places it inside a larger buffer to exercise copying. */
  private chunk(index: number): Uint8Array {
    const bytes = fakeChunkBytes(index)
    if (!this.engine.subViewChunks) {
      this.engine.drainedBuffers.push(bytes.buffer)
      return bytes
    }
    const backing = new Uint8Array(bytes.byteLength + 4)
    backing.set(bytes, 2)
    this.engine.drainedBuffers.push(backing.buffer)
    return backing.subarray(2, 2 + bytes.byteLength)
  }

  cancel(generation: number): void {
    this.engine.log.push(`cancel:${generation}`)
    this.exporting = false
    this.pending = []
  }

  finish(generation: number): EngineFrame {
    this.engine.log.push(`finish:${generation}`)
    const size = this.original ? this.engine.originalFrameSize : { width: 2, height: 1 }
    const frame = new FakeFrame(size.width, size.height)
    this.engine.frames.push(frame)
    return frame
  }

  dispose(): void {
    this.disposeCalls += 1
  }

  free(): void {
    this.freed = true
  }
}

/** Deterministic engine double with the same stage protocol as the WASM session. */
export class FakeEngine implements ProcessingEngine {
  readonly readyStage = FAKE_STAGES.ready
  readonly failedStage = FAKE_STAGES.failed
  readonly log: string[] = []
  readonly sessions: FakeSession[] = []
  readonly frames: FakeFrame[] = []
  /** Backing buffers of sub-view chunks, so tests can check that they were copied. */
  readonly drainedBuffers: ArrayBufferLike[] = []
  readonly steps: number
  readonly subViewChunks: boolean
  /** The advance index (1-based) whose export call throws `encode_failed`, or null. */
  readonly failExportAt: number | null
  failNextCreate: string | null = null
  failNextEncoded: string | null = null
  encodedMetadata = { width: 2, height: 1, channels: 4 }
  originalFrameSize = { width: 2, height: 1 }
  readonly encodedInputs: number[] = []
  onEncodedCreate: (() => void) | null = null
  onStart: ((generation: number) => void) | null = null
  /** Runs synchronously inside each advance, after its count is incremented. */
  onAdvance: ((advance: number) => void) | null = null
  private waiters: Array<() => void> = []

  constructor(
    options: {
      readonly steps?: number
      readonly subViewChunks?: boolean
      readonly failExportAt?: number
    } = {},
  ) {
    this.steps = options.steps ?? 3
    this.subViewChunks = options.subViewChunks ?? false
    this.failExportAt = options.failExportAt ?? null
  }

  createSession(width: number, height: number, pixels: Uint8Array): EngineSession {
    if (this.failNextCreate !== null) {
      const code = this.failNextCreate
      this.failNextCreate = null
      throw Object.assign(new Error('The fake engine rejected the source.'), { code })
    }
    if (pixels.byteLength !== width * height * 4) throw new Error('bad pixel length')
    const session = new FakeSession(this)
    this.sessions.push(session)
    return session
  }

  createEncodedSession(bytes: Uint8Array): EngineSession {
    this.encodedInputs.push(bytes.byteLength)
    if (this.failNextEncoded !== null) {
      const code = this.failNextEncoded
      this.failNextEncoded = null
      throw Object.assign(new Error('The fake engine could not decode the bytes.'), { code })
    }
    const { width, height, channels } = this.encodedMetadata
    const session = new FakeSession(this, width, height, channels)
    this.sessions.push(session)
    this.onEncodedCreate?.()
    return session
  }

  nextAdvance(): Promise<void> {
    return new Promise<void>((resolve) => {
      this.waiters.push(resolve)
    })
  }

  notifyAdvance(): void {
    const waiters = this.waiters
    this.waiters = []
    for (const resolve of waiters) resolve()
  }
}

export function portEndpoint(port: MessagePort): HostEndpoint {
  return {
    postMessage: (message, transfer) => port.postMessage(message, transfer),
    addEventListener: (type, listener) => {
      port.addEventListener(type, listener)
      if (type === 'message') port.start()
    },
    removeEventListener: (type, listener) => port.removeEventListener(type, listener),
  }
}

export interface LinkedPair {
  readonly client: ProcessingClient
  readonly host: WorkerHost
  readonly hostPort: MessagePort
  readonly clientPort: MessagePort
  close(): void
}

/** Connects a real client to a real host over a MessageChannel with structured-clone transfer. */
export function createLinkedPair(
  engine: ProcessingEngine,
  options: {
    readonly maxSources?: number
    readonly loadEngine?: () => Promise<ProcessingEngine>
  } = {},
): LinkedPair {
  const channel = new MessageChannel()
  const host = createWorkerHost(
    portEndpoint(channel.port1),
    options.loadEngine ?? (() => Promise.resolve(engine)),
    { pixelBudget: 1 },
  )
  const client = createProcessingClient({
    createEndpoint: () => ({
      ...portEndpoint(channel.port2),
      terminate: () => channel.port2.close(),
    }),
    ...(options.maxSources === undefined ? {} : { maxSources: options.maxSources }),
  })
  return {
    client,
    host,
    hostPort: channel.port1,
    clientPort: channel.port2,
    close(): void {
      client.dispose()
      host.dispose()
      channel.port1.close()
      channel.port2.close()
    },
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null
}

/** Client endpoint whose inbound events and outbound posts are driven by the test. */
export class ControlledEndpoint implements ClientEndpoint {
  readonly posted: Array<{ readonly message: unknown; readonly transfer: Transferable[] }> = []
  terminated = 0
  private readonly listeners = new Map<string, Set<(event: Event) => void>>()

  postMessage(message: unknown, transfer: Transferable[]): void {
    this.posted.push({ message, transfer })
  }

  addEventListener(type: string, listener: (event: Event) => void): void {
    const set = this.listeners.get(type) ?? new Set()
    set.add(listener)
    this.listeners.set(type, set)
  }

  removeEventListener(type: string, listener: (event: Event) => void): void {
    this.listeners.get(type)?.delete(listener)
  }

  terminate(): void {
    this.terminated += 1
  }

  listenerCount(): number {
    let count = 0
    for (const set of this.listeners.values()) count += set.size
    return count
  }

  emit(type: string, event: Event): void {
    for (const listener of [...(this.listeners.get(type) ?? [])]) listener(event)
  }

  emitMessage(data: unknown): void {
    this.emit('message', Object.assign(new Event('message'), { data }))
  }

  requests(type: string): Array<Record<string, unknown>> {
    const found: Array<Record<string, unknown>> = []
    for (const { message } of this.posted) {
      const record = asRecord(message)
      if (record !== null && record.type === type) found.push(record)
    }
    return found
  }
}

/** Completes an import on a controlled endpoint by replaying a matching worker reply. */
export async function importViaEndpoint(
  client: ProcessingClient,
  endpoint: ControlledEndpoint,
  operationId: number,
  sourceId: number,
) {
  const before = endpoint.requests('import').length
  const pending = client.importRgba(rgba(), operationId)
  await until(() => endpoint.requests('import').length > before, 'import request')
  const request = endpoint.requests('import').at(-1)
  endpoint.emitMessage({
    type: 'imported',
    requestId: request?.requestId,
    operationId,
    sourceId,
    name: 'test',
    width: 2,
    height: 1,
  })
  return pending
}
