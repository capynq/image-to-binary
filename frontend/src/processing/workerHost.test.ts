import { afterEach, describe, expect, it } from 'vitest'
import { eventData } from './protocol'
import { FakeEngine, portEndpoint, until } from './testKit'
import { createWorkerHost } from './workerHost'

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
): RawHost {
  const channel = new MessageChannel()
  const host = createWorkerHost(portEndpoint(channel.port1), loadEngine)
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
    parameters: { cellSize: 12, fontSize: 16, threshold: 128, sampling: 'classic' },
  })
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
