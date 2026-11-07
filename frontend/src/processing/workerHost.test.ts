import { afterEach, describe, expect, it } from 'vitest'
import { DEFAULT_RAMP, MAX_ENCODED_BYTES, eventData } from './protocol'
import type { TaskYielder } from './taskYield'
import { FakeEngine, portEndpoint, until } from './testKit'
import { createWorkerHost } from './workerHost'
import type { WorkerHostOptions } from './workerHost'

interface RawHost {
  readonly replies: unknown[]
  send(message: Record<string, unknown>, transfer?: Transferable[]): void
  close(): void
}

const open: RawHost[] = []

afterEach(() => {
  for (const raw of open.splice(0)) raw.close()
})

function openHost(
  engine: FakeEngine,
  loadEngine: () => Promise<FakeEngine> = () => Promise.resolve(engine),
  options: WorkerHostOptions = {},
): RawHost {
  const channel = new MessageChannel()
  const host = createWorkerHost(portEndpoint(channel.port1), loadEngine, options)
  const replies: unknown[] = []
  channel.port2.addEventListener('message', (event) => {
    replies.push(eventData(event))
  })
  channel.port2.start()
  const raw: RawHost = {
    replies,
    send: (message, transfer = []) => channel.port2.postMessage(message, transfer),
    close: () => {
      host.dispose()
      channel.port1.close()
      channel.port2.close()
    },
  }
  open.push(raw)
  return raw
}

function sendImport(raw: RawHost, requestId: number): void {
  const pixels = new ArrayBuffer(8)
  raw.send(
    { type: 'import', requestId, operationId: requestId, name: 'n', width: 2, height: 1, pixels },
    [pixels],
  )
}

function sendPreview(raw: RawHost, requestId: number, sourceId: number): void {
  raw.send({
    type: 'preview',
    requestId,
    operationId: requestId,
    sourceId,
    parameters: {
      cellSize: 12,
      fontSize: 16,
      threshold: 128,
      sampling: 'classic',
      mode: 'binary',
      ramp: DEFAULT_RAMP,
      foreground: '#00ff00',
      background: '#000000',
      colorMode: 'monochrome',
      contrast: 0,
      invert: false,
    },
  })
}

function sendImportEncoded(
  raw: RawHost,
  requestId: number,
  name = 'photo.png',
  bytes: ArrayBuffer = new ArrayBuffer(16),
): void {
  raw.send({ type: 'importEncoded', requestId, operationId: requestId, name, bytes }, [bytes])
}

function sendOriginal(raw: RawHost, requestId: number, sourceId: number): void {
  raw.send({ type: 'originalPreview', requestId, operationId: requestId, sourceId })
}

interface Gate {
  armed: boolean
  readonly create: () => TaskYielder
  waiting(): number
  releaseAll(): void
}

function createGate(): Gate {
  const pending: Array<() => void> = []
  const gate: Gate = {
    armed: false,
    create: (): TaskYielder => ({
      yield: () =>
        gate.armed
          ? new Promise<void>((resolve) => {
              pending.push(resolve)
            })
          : Promise.resolve(),
      close: () => gate.releaseAll(),
    }),
    waiting: () => pending.length,
    releaseAll: () => {
      gate.armed = false
      for (const resolve of pending.splice(0)) resolve()
    },
  }
  return gate
}

async function awaitReplies(raw: RawHost, count: number): Promise<void> {
  await until(() => raw.replies.length >= count, `${count} replies`)
}

async function roundTrip(raw: RawHost, send: () => void): Promise<void> {
  const expected = raw.replies.length + 1
  send()
  await awaitReplies(raw, expected)
}

describe('worker host request id ordering', () => {
  it('rejects a replayed import id after completion without creating a session', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    sendImport(raw, 1)
    await awaitReplies(raw, 1)
    expect(raw.replies[0]).toMatchObject({ type: 'imported', sourceId: 1 })
    sendImport(raw, 1)
    await awaitReplies(raw, 2)
    expect(raw.replies[1]).toMatchObject({ type: 'fatal' })
    expect(engine.sessions).toHaveLength(1)
  })

  it('rejects a lower import id without creating a session', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    sendImport(raw, 5)
    await awaitReplies(raw, 1)
    sendImport(raw, 3)
    await awaitReplies(raw, 2)
    expect(raw.replies[1]).toMatchObject({ type: 'fatal' })
    expect(engine.sessions).toHaveLength(1)
  })

  it('rejects a replayed preview id without starting more work', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImport(raw, 1))
    expect(raw.replies[0]).toMatchObject({ type: 'imported', requestId: 1 })
    await roundTrip(raw, () => sendPreview(raw, 2, 1))
    expect(raw.replies[1]).toMatchObject({ type: 'previewed', requestId: 2 })
    await roundTrip(raw, () => sendPreview(raw, 2, 1))
    expect(raw.replies[2]).toMatchObject({ type: 'fatal' })
    expect(engine.log.filter((entry) => entry.startsWith('start:'))).toEqual(['start:2'])
    expect(engine.frames).toHaveLength(1)
  })

  it('allows gaps left by replaced pending jobs', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImport(raw, 1))
    await roundTrip(raw, () => sendImport(raw, 4))
    await roundTrip(raw, () => sendPreview(raw, 9, 2))
    expect(raw.replies[0]).toMatchObject({ type: 'imported', requestId: 1, sourceId: 1 })
    expect(raw.replies[1]).toMatchObject({ type: 'imported', requestId: 4, sourceId: 2 })
    expect(raw.replies[2]).toMatchObject({ type: 'previewed', requestId: 9 })
    expect(engine.sessions).toHaveLength(2)
  })

  it('rejects a busy request without consuming its id', async () => {
    const engine = new FakeEngine()
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const raw = openHost(engine, async () => {
      await gate
      return engine
    })
    sendImport(raw, 1)
    await roundTrip(raw, () => sendImport(raw, 5))
    expect(raw.replies[0]).toMatchObject({ type: 'failed', requestId: 5, code: 'busy' })
    expect(engine.sessions).toHaveLength(0)

    release()
    await awaitReplies(raw, 2)
    expect(raw.replies[1]).toMatchObject({ type: 'imported', requestId: 1, sourceId: 1 })
    await roundTrip(raw, () => sendImport(raw, 2))
    expect(raw.replies[2]).toMatchObject({ type: 'imported', requestId: 2, sourceId: 2 })
    expect(raw.replies.some((reply) => (reply as { type?: unknown }).type === 'fatal')).toBe(false)
    expect(engine.sessions).toHaveLength(2)
  })

  it('still rejects a replay of the accepted id while busy', async () => {
    const engine = new FakeEngine()
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const raw = openHost(engine, async () => {
      await gate
      return engine
    })
    sendImport(raw, 3)
    await roundTrip(raw, () => sendImport(raw, 3))
    expect(raw.replies[0]).toMatchObject({ type: 'fatal' })
    release()
    expect(engine.sessions).toHaveLength(0)
  })

  it('does not treat cancel or release as scheduled request ids', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    sendImport(raw, 1)
    await awaitReplies(raw, 1)
    raw.send({ type: 'cancel', requestId: 1, operationId: 1 })
    raw.send({ type: 'releaseSource', sourceId: 99 })
    sendPreview(raw, 2, 1)
    await awaitReplies(raw, 2)
    expect(raw.replies[1]).toMatchObject({ type: 'previewed', requestId: 2 })
    expect(raw.replies.some((reply) => (reply as { type?: unknown }).type === 'fatal')).toBe(false)
  })
})

describe('worker host encoded import', () => {
  it('rejects invalid input before the engine is called', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImportEncoded(raw, 1, ''))
    await roundTrip(raw, () => sendImportEncoded(raw, 2, 'x'.repeat(257)))
    await roundTrip(raw, () => sendImportEncoded(raw, 3, 'n', new ArrayBuffer(0)))
    await roundTrip(raw, () =>
      sendImportEncoded(raw, 4, 'n', new ArrayBuffer(MAX_ENCODED_BYTES + 1)),
    )
    expect(raw.replies[0]).toMatchObject({ type: 'failed', requestId: 1, code: 'invalid_argument' })
    expect(raw.replies[1]).toMatchObject({ type: 'failed', requestId: 2, code: 'invalid_argument' })
    expect(raw.replies[2]).toMatchObject({ type: 'failed', requestId: 3, code: 'invalid_argument' })
    expect(raw.replies[3]).toMatchObject({
      type: 'failed',
      requestId: 4,
      code: 'resource_limit_exceeded',
    })
    expect(engine.encodedInputs).toEqual([])
    expect(engine.sessions).toHaveLength(0)
  })

  it('does not decode once the retained source limit is reached', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImport(raw, 1))
    await roundTrip(raw, () => sendImport(raw, 2))
    await roundTrip(raw, () => sendImportEncoded(raw, 3))
    expect(raw.replies[2]).toMatchObject({
      type: 'failed',
      requestId: 3,
      code: 'resource_limit_exceeded',
    })
    expect(engine.encodedInputs).toEqual([])
  })

  it('replies with the engine-normalized metadata', async () => {
    const engine = new FakeEngine()
    engine.encodedMetadata = { width: 3, height: 5, channels: 4 }
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImportEncoded(raw, 1, 'photo.png', new ArrayBuffer(16)))
    expect(raw.replies[0]).toMatchObject({
      type: 'imported',
      requestId: 1,
      sourceId: 1,
      name: 'photo.png',
      width: 3,
      height: 5,
    })
    expect(engine.encodedInputs).toEqual([16])
    expect(engine.sessions[0]?.freed).toBe(false)
  })

  it.each([
    ['non-RGBA channels', { width: 2, height: 1, channels: 3 }],
    ['zero width', { width: 0, height: 1, channels: 4 }],
    ['fractional height', { width: 2, height: 1.5, channels: 4 }],
    ['over 50MP', { width: 10_000, height: 10_000, channels: 4 }],
  ])('frees the session when the metadata has %s', async (_name, metadata) => {
    const engine = new FakeEngine()
    engine.encodedMetadata = metadata
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImportEncoded(raw, 1))
    expect(raw.replies[0]).toMatchObject({ type: 'failed', requestId: 1, code: 'internal' })
    expect(engine.sessions).toHaveLength(1)
    expect(engine.sessions[0]).toMatchObject({ disposeCalls: 1, freed: true })
    await roundTrip(raw, () => sendPreview(raw, 2, 1))
    expect(raw.replies[1]).toMatchObject({ type: 'failed', code: 'unknown_source' })
  })

  it('frees the new session and publishes nothing when cancelled after the decode yield', async () => {
    const engine = new FakeEngine()
    const gate = createGate()
    engine.onEncodedCreate = () => {
      gate.armed = true
    }
    const raw = openHost(engine, undefined, { createYielder: gate.create })
    sendImportEncoded(raw, 1)
    await until(() => gate.waiting() === 1, 'post-decode yield')
    expect(engine.sessions).toHaveLength(1)
    raw.send({ type: 'cancel', requestId: 1, operationId: 1 })
    await roundTrip(raw, () => sendImportEncoded(raw, 2))
    expect(raw.replies[0]).toMatchObject({ type: 'failed', requestId: 2, code: 'busy' })
    gate.releaseAll()
    await awaitReplies(raw, 2)
    expect(raw.replies[1]).toMatchObject({ type: 'cancelled', requestId: 1 })
    expect(raw.replies.some((reply) => (reply as { type?: unknown }).type === 'imported')).toBe(
      false,
    )
    expect(engine.sessions[0]).toMatchObject({ disposeCalls: 1, freed: true })
    await roundTrip(raw, () => sendPreview(raw, 3, 1))
    expect(raw.replies[2]).toMatchObject({ type: 'failed', code: 'unknown_source' })
  })

  it('preserves the accepted source when a later decode fails', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImport(raw, 1))
    engine.failNextEncoded = 'decode_failed'
    await roundTrip(raw, () => sendImportEncoded(raw, 2))
    expect(raw.replies[1]).toMatchObject({ type: 'failed', requestId: 2, code: 'decode_failed' })
    expect(engine.sessions).toHaveLength(1)
    expect(engine.sessions[0]).toMatchObject({ disposeCalls: 0, freed: false })
    await roundTrip(raw, () => sendPreview(raw, 3, 1))
    expect(raw.replies[2]).toMatchObject({ type: 'previewed', requestId: 3 })
  })

  it('keeps strict request id ordering for the new request types', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImportEncoded(raw, 1))
    await roundTrip(raw, () => sendOriginal(raw, 1, 1))
    expect(raw.replies[1]).toMatchObject({ type: 'fatal' })
    expect(engine.log).toEqual([])
  })
})

describe('worker host render modes', () => {
  it('passes the mode and ramp to the engine for previews and exports', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImport(raw, 1))
    const style = {
      foreground: '#112233',
      background: '#f0e0d0',
      colorMode: 'source',
      contrast: -40,
      invert: true,
    }
    const parameters = {
      cellSize: 12,
      fontSize: 16,
      threshold: 128,
      sampling: 'average',
      mode: 'ascii',
      ramp: ' .#@',
      ...style,
    }
    await roundTrip(raw, () =>
      raw.send({ type: 'preview', requestId: 2, operationId: 2, sourceId: 1, parameters }),
    )
    await roundTrip(raw, () =>
      raw.send({
        type: 'export',
        requestId: 3,
        operationId: 3,
        sourceId: 1,
        parameters: { ...parameters, mode: 'block', ramp: DEFAULT_RAMP },
      }),
    )
    expect(engine.starts).toEqual([
      {
        kind: 'preview',
        generation: 2,
        cellSize: 12,
        fontSize: 16,
        threshold: 128,
        sampling: 'average',
        mode: 'ascii',
        ramp: ' .#@',
        ...style,
      },
      {
        kind: 'export',
        generation: 3,
        cellSize: 12,
        fontSize: 16,
        threshold: 128,
        sampling: 'average',
        mode: 'block',
        ramp: DEFAULT_RAMP,
        ...style,
      },
    ])
  })
})

describe('worker host original preview', () => {
  it('drives the original job with bounded advances and a transferred owned frame', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImport(raw, 1))
    await roundTrip(raw, () => sendOriginal(raw, 2, 1))
    const reply = raw.replies[1] as {
      type: string
      width: number
      height: number
      pixels: ArrayBuffer
    }
    expect(reply).toMatchObject({ type: 'previewed', requestId: 2, width: 2, height: 1 })
    expect(reply.pixels.byteLength).toBe(8)
    expect(engine.log).toEqual(['original:2', 'finish:2'])
    expect(engine.sessions[0]?.budgets).toEqual([131_072, 131_072, 131_072])
    expect(engine.frames).toHaveLength(1)
    expect(engine.frames[0]).toMatchObject({ taken: true, freed: true })
  })

  it('fails an unknown source without starting a job', async () => {
    const engine = new FakeEngine()
    const raw = openHost(engine)
    await roundTrip(raw, () => sendOriginal(raw, 1, 7))
    expect(raw.replies[0]).toMatchObject({ type: 'failed', requestId: 1, code: 'unknown_source' })
    expect(engine.log).toEqual([])
  })

  it('rejects an oversized engine frame and still frees it', async () => {
    const engine = new FakeEngine()
    engine.originalFrameSize = { width: 1601, height: 1 }
    const raw = openHost(engine)
    await roundTrip(raw, () => sendImport(raw, 1))
    await roundTrip(raw, () => sendOriginal(raw, 2, 1))
    expect(raw.replies[1]).toMatchObject({ type: 'failed', requestId: 2, code: 'internal' })
    expect(engine.frames[0]).toMatchObject({ taken: true, freed: true })
  })

  it('publishes no frame when cancelled and keeps the source usable', async () => {
    const engine = new FakeEngine()
    const gate = createGate()
    const raw = openHost(engine, undefined, { createYielder: gate.create })
    await roundTrip(raw, () => sendImport(raw, 1))
    engine.onStart = () => {
      gate.armed = true
      engine.onStart = null
    }
    sendOriginal(raw, 2, 1)
    await until(() => gate.waiting() === 1, 'original advance yield')
    raw.send({ type: 'cancel', requestId: 2, operationId: 2 })
    await roundTrip(raw, () => sendOriginal(raw, 3, 1))
    expect(raw.replies[1]).toMatchObject({ type: 'failed', requestId: 3, code: 'busy' })
    gate.releaseAll()
    await awaitReplies(raw, 3)
    expect(raw.replies[2]).toMatchObject({ type: 'cancelled', requestId: 2 })
    expect(engine.log).toEqual(['original:2', 'cancel:2'])
    expect(engine.frames).toHaveLength(0)
    expect(engine.sessions[0]).toMatchObject({ disposeCalls: 0, freed: false })
    await roundTrip(raw, () => sendOriginal(raw, 3, 1))
    expect(raw.replies[3]).toMatchObject({ type: 'previewed', requestId: 3 })
  })

  it('defers disposal of a released source until the active original job unwinds', async () => {
    const engine = new FakeEngine()
    const gate = createGate()
    const raw = openHost(engine, undefined, { createYielder: gate.create })
    await roundTrip(raw, () => sendImport(raw, 1))
    engine.onStart = () => {
      gate.armed = true
      engine.onStart = null
    }
    sendOriginal(raw, 2, 1)
    await until(() => gate.waiting() === 1, 'original advance yield')
    raw.send({ type: 'releaseSource', sourceId: 1 })
    await roundTrip(raw, () => sendOriginal(raw, 3, 1))
    expect(raw.replies[1]).toMatchObject({ type: 'failed', requestId: 3, code: 'busy' })
    expect(engine.sessions[0]).toMatchObject({ disposeCalls: 0, freed: false })
    gate.releaseAll()
    await awaitReplies(raw, 3)
    expect(raw.replies[2]).toMatchObject({ type: 'cancelled', requestId: 2 })
    expect(engine.frames).toHaveLength(0)
    expect(engine.log).toEqual(['original:2'])
    expect(engine.sessions[0]).toMatchObject({ disposeCalls: 1, freed: true })
  })
})
