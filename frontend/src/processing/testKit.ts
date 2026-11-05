import type { EngineFrame, EngineSession, ProcessingEngine } from './engine'
import type { PreviewParameters } from './protocol'
import { createProcessingClient } from './workerClient'
import type { ClientEndpoint, ProcessingClient, RgbaImport } from './workerClient'
import { createWorkerHost } from './workerHost'
import type { HostEndpoint, WorkerHost } from './workerHost'

export const FAKE_STAGES = { sampling: 1, preview: 2, ready: 3, failed: 4 } as const

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

export class FakeSession implements EngineSession {
  disposeCalls = 0
  freed = false
  advances = 0

  constructor(private readonly engine: FakeEngine) {}

  startPreview(generation: number): number {
    this.engine.log.push(`start:${generation}`)
    this.engine.onStart?.(generation)
    this.advances = 0
    return FAKE_STAGES.sampling
  }

  advance(): number {
    this.advances += 1
    this.engine.notifyAdvance()
    if (this.advances >= this.engine.steps) return FAKE_STAGES.ready
    return this.advances * 2 >= this.engine.steps ? FAKE_STAGES.preview : FAKE_STAGES.sampling
  }

  cancel(generation: number): void {
    this.engine.log.push(`cancel:${generation}`)
  }

  finish(generation: number): EngineFrame {
    this.engine.log.push(`finish:${generation}`)
    const frame = new FakeFrame(2, 1)
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
  readonly steps: number
  failNextCreate: string | null = null
  onStart: ((generation: number) => void) | null = null
  private waiters: Array<() => void> = []

  constructor(options: { readonly steps?: number } = {}) {
    this.steps = options.steps ?? 3
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
