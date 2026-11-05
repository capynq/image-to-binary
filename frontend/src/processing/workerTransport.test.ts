import { afterEach, describe, expect, it } from 'vitest'
import { eventData } from './protocol'
import { TASK_YIELD_TIMER_INTERVAL } from './taskYield'
import { createProcessingClient, ProcessingWorkerError } from './workerClient'
import type { PreviewPixels, ProcessingClient, Releasable, SourceHandle } from './workerClient'
import { createWorkerHost } from './workerHost'
import {
  ControlledEndpoint,
  FakeEngine,
  PARAMS,
  createLinkedPair,
  importViaEndpoint,
  rgba,
  settle,
  until,
} from './testKit'
import type { LinkedPair } from './testKit'

const open: LinkedPair[] = []

function track(pair: LinkedPair): LinkedPair {
  open.push(pair)
  return pair
}

afterEach(() => {
  for (const pair of open.splice(0)) pair.close()
})

describe('worker transport over MessageChannel', () => {
  it('imports owned RGBA, detaches the input and previews an owned buffer', async () => {
    const engine = new FakeEngine({ steps: 4 })
    const { client } = track(createLinkedPair(engine))
    const pixels = new ArrayBuffer(8)
    const handle = await client.importRgba({ name: 'tiny', width: 2, height: 1, pixels }, 1)
    expect(pixels.byteLength).toBe(0)
    expect(handle).toMatchObject({ name: 'tiny', width: 2, height: 1 })

    const result = await client.previewPixels(handle, PARAMS, 2)
    expect(result.width).toBe(2)
    expect(result.height).toBe(1)
    expect(result.pixels.byteLength).toBe(8)
    expect(new Uint8Array(result.pixels)[0]).toBe(7)
    expect(engine.frames.map((frame) => frame.freed)).toEqual([true])
    expect(engine.log).toEqual(['start:2', 'finish:2'])
  })

  it('validates before transfer and creates no worker for invalid input', async () => {
    let created = 0
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({
      createEndpoint: () => {
        created += 1
        return endpoint
      },
    })
    const short = new ArrayBuffer(3)
    await expect(
      client.importRgba({ name: 'bad', width: 2, height: 1, pixels: short }, 1),
    ).rejects.toThrow('Pixel buffer length')
    expect(short.byteLength).toBe(3)
    await expect(
      client.importRgba(
        { name: 'big', width: 10_000, height: 10_000, pixels: new ArrayBuffer(8) },
        2,
      ),
    ).rejects.toThrow('pixel limit')
    expect(created).toBe(0)
    expect(endpoint.posted).toHaveLength(0)
    client.dispose()
  })

  it('creates no endpoint until used', () => {
    let created = 0
    const client = createProcessingClient({
      createEndpoint: () => {
        created += 1
        return new ControlledEndpoint()
      },
    })
    client.dispose()
    expect(created).toBe(0)
  })

  it('keeps one active job and the newest pending job, cancelling the worker first', async () => {
    const engine = new FakeEngine({ steps: 2000 })
    const { client } = track(createLinkedPair(engine))
    const handle = await client.importRgba(rgba(), 1)
    const started = engine.nextAdvance()
    const first = settle(client.previewPixels(handle, PARAMS, 2))
    await started
    const replaced = settle(client.previewPixels(handle, PARAMS, 3))
    const newest = settle(client.previewPixels(handle, PARAMS, 4))

    expect((await first).ok).toBe(false)
    expect((await replaced).ok).toBe(false)
    expect((await newest).ok).toBe(true)
    expect(engine.log.filter((entry) => entry.startsWith('start:'))).toEqual(['start:2', 'start:4'])
    expect(engine.log).not.toContain('finish:2')
    const cancelled = engine.log.indexOf('cancel:2')
    expect(cancelled).toBeGreaterThan(-1)
    expect(cancelled).toBeLessThan(engine.log.indexOf('start:4'))
    expect(engine.frames).toHaveLength(1)
  })

  it('cancel(operationId) stops worker computation without publishing a partial frame', async () => {
    const engine = new FakeEngine({ steps: 2000 })
    const { client } = track(createLinkedPair(engine))
    const handle = await client.importRgba(rgba(), 1)
    const started = engine.nextAdvance()
    const run = settle(client.previewPixels(handle, PARAMS, 2))
    await started
    client.cancel(2)
    expect((await run).ok).toBe(false)
    await until(() => engine.log.includes('cancel:2'), 'worker cancel')
    expect(engine.frames).toHaveLength(0)

    const next = await client.previewPixels(handle, PARAMS, 3)
    expect(next.width).toBe(2)
    expect(engine.log.indexOf('cancel:2')).toBeLessThan(engine.log.indexOf('start:3'))
  })

  it('releases sources deterministically and rejects released or foreign handles', async () => {
    const engineA = new FakeEngine()
    const engineB = new FakeEngine()
    const a = track(createLinkedPair(engineA))
    const b = track(createLinkedPair(engineB))
    const handle = await a.client.importRgba(rgba(), 1)
    await expect(b.client.previewPixels(handle, PARAMS, 1)).rejects.toThrow('source handle')

    handle.dispose()
    handle.dispose()
    await until(() => engineA.sessions[0]?.disposeCalls === 1, 'session release')
    expect(engineA.sessions[0]?.freed).toBe(true)
    await expect(a.client.previewPixels(handle, PARAMS, 2)).rejects.toThrow('source handle')
  })

  it('bounds retained sources', async () => {
    const engine = new FakeEngine()
    const { client } = track(createLinkedPair(engine, { maxSources: 2 }))
    const first = await client.importRgba(rgba(), 1)
    await client.importRgba(rgba(), 2)
    await expect(client.importRgba(rgba(), 3)).rejects.toThrow('retained')
    first.dispose()
    const third = await client.importRgba(rgba(), 4)
    expect(third.sourceId).toBeGreaterThan(first.sourceId)
  })

  it('releases a source while it is previewing and publishes nothing', async () => {
    const engine = new FakeEngine({ steps: 2000 })
    const { client } = track(createLinkedPair(engine))
    const handle = await client.importRgba(rgba(), 1)
    const started = engine.nextAdvance()
    const run = settle(client.previewPixels(handle, PARAMS, 2))
    await started
    handle.dispose()
    expect((await run).ok).toBe(false)
    await until(() => engine.sessions[0]?.disposeCalls === 1, 'deferred session release')
    expect(engine.frames).toHaveLength(0)
    expect(engine.log.some((entry) => entry.startsWith('finish:'))).toBe(false)
  })

  it('surfaces stable engine error codes without failing the session', async () => {
    const engine = new FakeEngine()
    const { client } = track(createLinkedPair(engine))
    engine.failNextCreate = 'invalid_dimensions'
    const failed = await settle(client.importRgba(rgba(), 1))
    expect(failed.ok).toBe(false)
    if (!failed.ok) {
      expect(failed.error).toBeInstanceOf(ProcessingWorkerError)
      expect((failed.error as ProcessingWorkerError).code).toBe('invalid_dimensions')
    }
    const handle = await client.importRgba(rgba(), 2)
    expect(handle.width).toBe(2)
  })

  it('fails the session deterministically when the engine cannot load', async () => {
    const engine = new FakeEngine()
    const { client } = track(
      createLinkedPair(engine, { loadEngine: () => Promise.reject(new Error('no wasm')) }),
    )
    await expect(client.importRgba(rgba(), 1)).rejects.toThrow('failed to load')
    await expect(client.importRgba(rgba(), 2)).rejects.toThrow('failed to load')
  })

  it('answers a malformed request with a fatal reply and stops', async () => {
    const channel = new MessageChannel()
    const host = createWorkerHost(
      {
        postMessage: (message, transfer) => channel.port1.postMessage(message, transfer),
        addEventListener: (type, listener) => {
          channel.port1.addEventListener(type, listener)
          channel.port1.start()
        },
        removeEventListener: (type, listener) => channel.port1.removeEventListener(type, listener),
      },
      () => Promise.resolve(new FakeEngine()),
    )
    const reply = new Promise<unknown>((resolve) => {
      channel.port2.addEventListener('message', (event) => resolve(eventData(event)))
      channel.port2.start()
    })
    channel.port2.postMessage({ type: 'bogus' })
    expect(await reply).toMatchObject({ type: 'fatal' })
    host.dispose()
    channel.port1.close()
    channel.port2.close()
  })
})

describe('client failure handling with a controlled endpoint', () => {
  it('discards a late stale import by releasing its source', async () => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const importing = settle(client.importRgba(rgba(), 1))
    await until(() => endpoint.requests('import').length === 1, 'import request')
    const request = endpoint.requests('import')[0]
    client.cancel(1)
    expect((await importing).ok).toBe(false)
    expect(endpoint.requests('cancel')).toHaveLength(1)

    endpoint.emitMessage({
      type: 'imported',
      requestId: request?.requestId,
      operationId: 1,
      sourceId: 5,
      name: 'test',
      width: 2,
      height: 1,
    })
    await until(() => endpoint.requests('releaseSource').length === 1, 'source release')
    expect(endpoint.requests('releaseSource')[0]?.sourceId).toBe(5)
    client.dispose()
  })

  it('skips finalizing a stale preview reply and then runs the newest job', async () => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const handle = await importViaEndpoint(client, endpoint, 1, 1)
    let finalized = 0
    const first = settle(
      client.preview(handle, PARAMS, 2, () => {
        finalized += 1
        return Promise.resolve({ dispose: () => undefined })
      }),
    )
    await until(() => endpoint.requests('preview').length === 1, 'first preview')
    const second = settle(client.previewPixels(handle, PARAMS, 3))
    expect((await first).ok).toBe(false)
    expect(endpoint.requests('cancel')).toHaveLength(1)

    const firstRequest = endpoint.requests('preview')[0]
    endpoint.emitMessage({
      type: 'previewed',
      requestId: firstRequest?.requestId,
      operationId: 2,
      width: 2,
      height: 1,
      pixels: new ArrayBuffer(8),
    })
    await until(() => endpoint.requests('preview').length === 2, 'newest preview')
    const secondRequest = endpoint.requests('preview')[1]
    endpoint.emitMessage({
      type: 'previewed',
      requestId: secondRequest?.requestId,
      operationId: 3,
      width: 2,
      height: 1,
      pixels: new ArrayBuffer(8),
    })
    expect((await second).ok).toBe(true)
    expect(finalized).toBe(0)
    client.dispose()
  })

  it('fails every outstanding request on a worker error and stays failed', async () => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const handle = await importViaEndpoint(client, endpoint, 1, 1)
    const first = settle(client.previewPixels(handle, PARAMS, 2))
    await until(() => endpoint.requests('preview').length === 1, 'preview request')
    const second = settle(client.previewPixels(handle, PARAMS, 3))
    endpoint.emit('error', Object.assign(new Event('error'), { message: 'worker script failed' }))

    expect((await first).ok).toBe(false)
    const secondResult = await second
    expect(secondResult.ok).toBe(false)
    if (!secondResult.ok) expect(secondResult.error.message).toContain('worker script failed')
    expect(endpoint.terminated).toBe(1)
    expect(endpoint.listenerCount()).toBe(0)
    await expect(client.previewPixels(handle, PARAMS, 4)).rejects.toThrow('worker script failed')
    await expect(client.importRgba(rgba(), 5)).rejects.toThrow('worker script failed')
  })

  it('fails the session on messageerror', async () => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const importing = settle(client.importRgba(rgba(), 1))
    await until(() => endpoint.requests('import').length === 1, 'import request')
    endpoint.emit('messageerror', new Event('messageerror'))
    const result = await importing
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error.message).toContain('deserialized')
    expect(endpoint.terminated).toBe(1)
  })

  it.each([
    ['non-object', 'garbage'],
    ['unknown type', { type: 'surprise' }],
    ['uncorrelated id', { type: 'cancelled', requestId: 999, operationId: 1 }],
    ['wrong operation', { type: 'cancelled', requestId: 1, operationId: 77 }],
    [
      'wrong kind',
      {
        type: 'previewed',
        requestId: 1,
        operationId: 1,
        width: 1,
        height: 1,
        pixels: new ArrayBuffer(4),
      },
    ],
  ])('treats a %s reply as fatal without hanging', async (_name, reply) => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const importing = settle(client.importRgba(rgba(), 1))
    await until(() => endpoint.requests('import').length === 1, 'import request')
    endpoint.emitMessage(reply)
    expect((await importing).ok).toBe(false)
    expect(endpoint.terminated).toBe(1)
  })

  it('rejects fatal worker replies', async () => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const importing = settle(client.importRgba(rgba(), 1))
    await until(() => endpoint.requests('import').length === 1, 'import request')
    endpoint.emitMessage({ type: 'fatal', message: 'engine exploded' })
    const result = await importing
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error.message).toBe('engine exploded')
  })

  it('handles dispose during init and afterwards idempotently', async () => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const importing = settle(client.importRgba(rgba(), 1))
    await until(() => endpoint.requests('import').length === 1, 'import request')
    client.dispose()
    client.dispose()
    const result = await importing
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error.message).toContain('disposed')
    expect(endpoint.terminated).toBe(1)
    expect(endpoint.listenerCount()).toBe(0)
    const posts = endpoint.posted.length
    endpoint.emitMessage({ type: 'fatal', message: 'ignored' })
    await expect(client.importRgba(rgba(), 2)).rejects.toThrow('disposed')
    expect(endpoint.posted).toHaveLength(posts)
  })

  it('fails when the endpoint cannot be created', async () => {
    const client = createProcessingClient({
      createEndpoint: () => {
        throw new Error('workers unsupported')
      },
    })
    await expect(client.importRgba(rgba(), 1)).rejects.toThrow('workers unsupported')
    await expect(client.importRgba(rgba(), 2)).rejects.toThrow('workers unsupported')
  })
})

describe('malformed worker replies fail the session without returning bogus results', () => {
  it.each([
    ['a zero source id', { sourceId: 0, name: 'test', width: 2, height: 1 }],
    ['a fractional source id', { sourceId: 1.5, name: 'test', width: 2, height: 1 }],
    ['an empty name', { sourceId: 1, name: '', width: 2, height: 1 }],
    ['zero dimensions', { sourceId: 1, name: 'test', width: 0, height: 1 }],
    ['over 50MP dimensions', { sourceId: 1, name: 'test', width: 10_000, height: 10_000 }],
    ['metadata differing from the request', { sourceId: 1, name: 'test', width: 3, height: 1 }],
  ])('rejects an imported reply with %s', async (_name, fields) => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const importing = settle(client.importRgba(rgba(), 1))
    await until(() => endpoint.requests('import').length === 1, 'import request')
    const request = endpoint.requests('import')[0]
    endpoint.emitMessage({
      type: 'imported',
      requestId: request?.requestId,
      operationId: 1,
      ...fields,
    })
    expect((await importing).ok).toBe(false)
    expect(endpoint.terminated).toBe(1)
    await expect(client.importRgba(rgba(), 2)).rejects.toThrow()
    expect(endpoint.requests('import')).toHaveLength(1)
  })

  it('refuses to overwrite a live source handle with a duplicate id', async () => {
    const endpoint = new ControlledEndpoint()
    const client = createProcessingClient({ createEndpoint: () => endpoint })
    const handle = await importViaEndpoint(client, endpoint, 1, 7)
    const second = settle(client.importRgba(rgba(), 2))
    await until(() => endpoint.requests('import').length === 2, 'second import request')
    endpoint.emitMessage({
      type: 'imported',
      requestId: endpoint.requests('import')[1]?.requestId,
      operationId: 2,
      sourceId: 7,
      name: 'test',
      width: 2,
      height: 1,
    })
    expect((await second).ok).toBe(false)
    expect(endpoint.terminated).toBe(1)
    await expect(client.previewPixels(handle, PARAMS, 3)).rejects.toThrow()
  })

  it.each([
    ['an oversized width', 1601, 1, 1601 * 4],
    ['an oversized height', 1, 1601, 1601 * 4],
    ['zero width', 0, 1, 0],
    ['a short buffer', 2, 1, 7],
    ['a long buffer', 2, 1, 9],
  ])(
    'rejects a previewed reply with %s without finalizing',
    async (_name, width, height, bytes) => {
      const endpoint = new ControlledEndpoint()
      const client = createProcessingClient({ createEndpoint: () => endpoint })
      const handle = await importViaEndpoint(client, endpoint, 1, 1)
      let finalized = 0
      const run = settle(
        client.preview(handle, PARAMS, 2, () => {
          finalized += 1
          return Promise.resolve({ dispose: () => undefined })
        }),
      )
      await until(() => endpoint.requests('preview').length === 1, 'preview request')
      endpoint.emitMessage({
        type: 'previewed',
        requestId: endpoint.requests('preview')[0]?.requestId,
        operationId: 2,
        width,
        height,
        pixels: new ArrayBuffer(bytes),
      })
      expect((await run).ok).toBe(false)
      expect(finalized).toBe(0)
      expect(endpoint.terminated).toBe(1)
    },
  )
})

describe('cancellation around worker initialization and the first advance', () => {
  it('rejects the caller promptly while the physical job keeps its slot until acknowledged', async () => {
    const engine = new FakeEngine()
    let loads = 0
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const { client } = track(
      createLinkedPair(engine, {
        loadEngine: async () => {
          loads += 1
          await gate
          return engine
        },
      }),
    )
    const first = settle(client.importRgba(rgba(), 1))
    await until(() => loads === 1, 'engine initialization')
    client.cancel(1)
    expect((await first).ok).toBe(false)

    const second = settle(client.importRgba(rgba(), 2))
    await new Promise<void>((resolve) => setTimeout(resolve, 10))
    expect(engine.sessions).toHaveLength(0)

    release()
    const result = await second
    expect(result.ok).toBe(true)
    expect(engine.sessions).toHaveLength(1)
    expect(loads).toBe(1)
  })

  it('terminates and settles everything when disposed during initialization', async () => {
    const engine = new FakeEngine()
    let loads = 0
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const pair = track(
      createLinkedPair(engine, {
        loadEngine: async () => {
          loads += 1
          await gate
          return engine
        },
      }),
    )
    const first = settle(pair.client.importRgba(rgba(), 1))
    await until(() => loads === 1, 'engine initialization')
    const queued = settle(pair.client.importRgba(rgba(), 2))
    pair.client.dispose()
    pair.host.dispose()

    const firstResult = await first
    const queuedResult = await queued
    expect(firstResult.ok).toBe(false)
    expect(queuedResult.ok).toBe(false)
    if (!firstResult.ok) expect(firstResult.error.message).toContain('disposed')
    if (!queuedResult.ok) expect(queuedResult.error.message).toContain('disposed')

    release()
    await new Promise<void>((resolve) => setTimeout(resolve, 10))
    expect(engine.sessions).toHaveLength(0)
  })

  it('cancels within the documented yield interval without publishing a frame', async () => {
    const engine = new FakeEngine({ steps: 2000 })
    const { client } = track(createLinkedPair(engine))
    const handle = await client.importRgba(rgba(), 1)
    engine.onStart = () => client.cancel(2)
    const run = settle(client.previewPixels(handle, PARAMS, 2))
    expect((await run).ok).toBe(false)
    engine.onStart = null
    await until(() => engine.log.includes('cancel:2'), 'worker cancel')
    expect(engine.sessions[0]?.advances ?? 0).toBeLessThanOrEqual(TASK_YIELD_TIMER_INTERVAL)
    expect(engine.log.filter((entry) => entry.startsWith('start:'))).toEqual(['start:2'])
    expect(engine.log.some((entry) => entry.startsWith('finish:'))).toBe(false)
    expect(engine.frames).toHaveLength(0)

    const next = await client.previewPixels(handle, PARAMS, 3)
    expect(next.width).toBe(2)
    expect(engine.log.indexOf('cancel:2')).toBeLessThan(engine.log.indexOf('start:3'))
  })
})

type PreviewKind = 'preview' | 'originalPreview'

/** Starts a regular or original preview through the same client entry points the adapter uses. */
function startPreview<T extends Releasable>(
  kind: PreviewKind,
  client: ProcessingClient,
  handle: SourceHandle,
  operationId: number,
  finalize: (pixels: PreviewPixels) => Promise<T>,
): Promise<T> {
  return kind === 'preview'
    ? client.preview(handle, PARAMS, operationId, finalize)
    : client.originalPreview(handle, operationId, finalize)
}

/** Replays a valid 2x1 `previewed` reply for the indexed request of `kind`. */
function emitPreviewed(
  endpoint: ControlledEndpoint,
  kind: PreviewKind,
  index: number,
  operationId: number,
): void {
  endpoint.emitMessage({
    type: 'previewed',
    requestId: endpoint.requests(kind)[index]?.requestId,
    operationId,
    width: 2,
    height: 1,
    pixels: new ArrayBuffer(8),
  })
}

/** Lets already-queued microtasks drain; no worker or timer work is pending when it is used. */
function flush(): Promise<void> {
  return new Promise<void>((resolve) => setTimeout(resolve, 0))
}

function countingClient(): {
  readonly client: ProcessingClient
  readonly endpoint: ControlledEndpoint
  readonly created: () => number
} {
  const endpoint = new ControlledEndpoint()
  let created = 0
  const client = createProcessingClient({
    createEndpoint: () => {
      created += 1
      return endpoint
    },
  })
  return { client, endpoint, created: () => created }
}

describe('encoded import and original preview transport', () => {
  it('imports encoded bytes over a linked pair, detaching them and returning decoded metadata', async () => {
    const engine = new FakeEngine()
    engine.encodedMetadata = { width: 3, height: 5, channels: 4 }
    const { client } = track(createLinkedPair(engine))
    const bytes = new ArrayBuffer(6)
    const handle = await client.importEncoded({ name: 'photo.png', bytes }, 1)
    expect(bytes.byteLength).toBe(0)
    expect(handle).toMatchObject({ name: 'photo.png', width: 3, height: 5 })
    expect(engine.encodedInputs).toEqual([6])
  })

  it('does not create an endpoint or transfer when an encoded producer is cancelled while awaiting', async () => {
    const { client, endpoint, created } = countingClient()
    const bytes = new ArrayBuffer(4)
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    let produced = 0
    const run = settle(
      client.importEncoded(async () => {
        produced += 1
        await gate
        return { name: 'a.png', bytes }
      }, 1),
    )
    await until(() => produced === 1, 'encoded producer')
    client.cancel(1)
    expect((await run).ok).toBe(false)
    release()
    await flush()
    expect(created()).toBe(0)
    expect(endpoint.posted).toHaveLength(0)
    expect(bytes.byteLength).toBe(4)
    client.dispose()
  })

  it('never invokes a queued encoded producer that is superseded before it becomes active', async () => {
    const { client, endpoint } = countingClient()
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    let first = 0
    let second = 0
    let third = 0
    const run1 = settle(
      client.importEncoded(async () => {
        first += 1
        await gate
        return { name: 'a.png', bytes: new ArrayBuffer(4) }
      }, 1),
    )
    await until(() => first === 1, 'active producer')
    const run2 = settle(
      client.importEncoded(() => {
        second += 1
        return { name: 'b.png', bytes: new ArrayBuffer(4) }
      }, 2),
    )
    const run3 = settle(
      client.importEncoded(() => {
        third += 1
        return { name: 'c.png', bytes: new ArrayBuffer(4) }
      }, 3),
    )
    expect((await run1).ok).toBe(false)
    expect((await run2).ok).toBe(false)

    release()
    await until(() => endpoint.requests('importEncoded').length === 1, 'newest encoded request')
    expect(second).toBe(0)
    expect(third).toBe(1)
    endpoint.emitMessage({
      type: 'imported',
      requestId: endpoint.requests('importEncoded')[0]?.requestId,
      operationId: 3,
      sourceId: 1,
      name: 'c.png',
      width: 2,
      height: 1,
    })
    expect((await run3).ok).toBe(true)
    client.dispose()
  })

  it('keeps one physical slot across an active original preview, a pending encoded import and a newer original preview', async () => {
    const { client, endpoint } = countingClient()
    const handle = await importViaEndpoint(client, endpoint, 1, 1)
    let produced = 0
    const active = settle(client.originalPreviewPixels(handle, 2))
    await until(() => endpoint.requests('originalPreview').length === 1, 'active original preview')
    const queued = settle(
      client.importEncoded(() => {
        produced += 1
        return { name: 'q.png', bytes: new ArrayBuffer(4) }
      }, 3),
    )
    const newest = settle(client.originalPreviewPixels(handle, 4))
    expect((await active).ok).toBe(false)
    expect((await queued).ok).toBe(false)
    expect(endpoint.requests('cancel')).toHaveLength(1)
    expect(endpoint.requests('originalPreview')).toHaveLength(1)

    endpoint.emitMessage({
      type: 'cancelled',
      requestId: endpoint.requests('originalPreview')[0]?.requestId,
      operationId: 2,
    })
    await until(() => endpoint.requests('originalPreview').length === 2, 'newest original preview')
    emitPreviewed(endpoint, 'originalPreview', 1, 4)
    expect((await newest).ok).toBe(true)

    expect(produced).toBe(0)
    expect(endpoint.requests('importEncoded')).toHaveLength(0)
    const ids = endpoint.posted.flatMap(({ message }) => {
      const record = message as { readonly type: string; readonly requestId: number }
      return record.type === 'import' || record.type === 'originalPreview' ? [record.requestId] : []
    })
    expect(ids).toHaveLength(3)
    expect(new Set(ids).size).toBe(3)
    expect(ids).toEqual([...ids].sort((a, b) => a - b))
    client.dispose()
  })

  it.each<PreviewKind>(['preview', 'originalPreview'])(
    'disposes a %s finalizer artifact completed after supersession exactly once and passes the newest',
    async (kind) => {
      const { client, endpoint } = countingClient()
      const handle = await importViaEndpoint(client, endpoint, 1, 1)
      let release: () => void = () => undefined
      const gate = new Promise<void>((resolve) => {
        release = resolve
      })
      let created = 0
      let disposals = 0
      const first = settle(
        startPreview(kind, client, handle, 2, async () => {
          created += 1
          await gate
          return {
            dispose: () => {
              disposals += 1
            },
          }
        }),
      )
      await until(() => endpoint.requests(kind).length === 1, 'first request')
      emitPreviewed(endpoint, kind, 0, 2)
      await until(() => created === 1, 'finalizer start')

      const newest = settle(
        startPreview(kind, client, handle, 3, (pixels) =>
          Promise.resolve({ ...pixels, dispose: () => undefined }),
        ),
      )
      expect((await first).ok).toBe(false)
      expect(disposals).toBe(0)
      expect(endpoint.requests(kind)).toHaveLength(1)

      release()
      await until(() => disposals === 1, 'artifact disposal')
      await until(() => endpoint.requests(kind).length === 2, 'newest request')
      emitPreviewed(endpoint, kind, 1, 3)
      expect((await newest).ok).toBe(true)
      expect(disposals).toBe(1)
      client.dispose()
    },
  )

  it.each<PreviewKind>(['preview', 'originalPreview'])(
    'skips the finalizer when a %s is cancelled before its reply arrives',
    async (kind) => {
      const { client, endpoint } = countingClient()
      const handle = await importViaEndpoint(client, endpoint, 1, 1)
      let finalized = 0
      const run = settle(
        startPreview(kind, client, handle, 2, () => {
          finalized += 1
          return Promise.resolve({ dispose: () => undefined })
        }),
      )
      await until(() => endpoint.requests(kind).length === 1, 'request')
      client.cancel(2)
      expect((await run).ok).toBe(false)
      emitPreviewed(endpoint, kind, 0, 2)
      await flush()
      expect(finalized).toBe(0)
      client.dispose()
    },
  )

  it.each([
    ['an oversized width', 1601, 1, 1601 * 4],
    ['an oversized height', 1, 1601, 1601 * 4],
    ['a wrong RGBA length', 2, 1, 7],
  ])(
    'fails the session on a malformed original preview reply with %s',
    async (_name, width, height, bytes) => {
      const { client, endpoint } = countingClient()
      const handle = await importViaEndpoint(client, endpoint, 1, 1)
      let finalized = 0
      const run = settle(
        client.originalPreview(handle, 2, () => {
          finalized += 1
          return Promise.resolve({ dispose: () => undefined })
        }),
      )
      await until(() => endpoint.requests('originalPreview').length === 1, 'original request')
      endpoint.emitMessage({
        type: 'previewed',
        requestId: endpoint.requests('originalPreview')[0]?.requestId,
        operationId: 2,
        width,
        height,
        pixels: new ArrayBuffer(bytes),
      })
      expect((await run).ok).toBe(false)
      expect(finalized).toBe(0)
      expect(endpoint.terminated).toBe(1)
    },
  )

  it.each<[string, string, () => ArrayBuffer, string]>([
    ['an empty name', '', () => new ArrayBuffer(4), 'Source name'],
    ['an overlong name', 'x'.repeat(257), () => new ArrayBuffer(4), 'Source name'],
    ['empty bytes', 'a.png', () => new ArrayBuffer(0), 'must not be empty'],
    ['a typed array', 'a.png', () => new Uint8Array(4) as unknown as ArrayBuffer, 'ArrayBuffer'],
  ])(
    'rejects encoded input with %s before creating an endpoint',
    async (_label, name, make, message) => {
      const { client, endpoint, created } = countingClient()
      const bytes = make()
      const length = bytes.byteLength
      await expect(client.importEncoded({ name, bytes }, 1)).rejects.toThrow(message)
      expect(bytes.byteLength).toBe(length)
      expect(created()).toBe(0)
      expect(endpoint.posted).toHaveLength(0)
      client.dispose()
    },
  )

  it('enforces the source cap for encoded imports without detaching the rejected bytes', async () => {
    const engine = new FakeEngine()
    const { client } = track(createLinkedPair(engine, { maxSources: 1 }))
    await client.importEncoded({ name: 'a.png', bytes: new ArrayBuffer(4) }, 1)
    const bytes = new ArrayBuffer(4)
    await expect(client.importEncoded({ name: 'b.png', bytes }, 2)).rejects.toThrow('retained')
    expect(bytes.byteLength).toBe(4)
    expect(engine.encodedInputs).toEqual([4])
  })

  it('rejects original previews for released or foreign handles without posting a request', async () => {
    const a = track(createLinkedPair(new FakeEngine()))
    const b = track(createLinkedPair(new FakeEngine()))
    const handle = await a.client.importEncoded({ name: 'a.png', bytes: new ArrayBuffer(4) }, 1)
    await expect(
      b.client.originalPreview(handle, 1, () => Promise.resolve({ dispose: () => undefined })),
    ).rejects.toThrow('source handle')
    handle.dispose()
    await expect(a.client.originalPreviewPixels(handle, 2)).rejects.toThrow('source handle')
  })
})
