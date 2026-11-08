import { describe, expect, it } from 'vitest'
import { createTaskYielder } from './taskYield'
import type { TaskYielder } from './taskYield'
import { createExportAccumulator } from './exportAccumulator'
import { createWorkerHost } from './workerHost'
import {
  ControlledEndpoint,
  createLinkedPair,
  FakeEngine,
  fakeChunkBytes,
  fakePngBytes,
  PARAMS,
  rgba,
  until,
} from './testKit'

interface YieldCounter {
  yields: number
}

function hostFixture(engine: FakeEngine) {
  const endpoint = new ControlledEndpoint()
  const counter: YieldCounter = { yields: 0 }
  const createYielder = (): TaskYielder => {
    const real = createTaskYielder()
    return {
      yield: () => {
        counter.yields += 1
        return real.yield()
      },
      close: () => real.close(),
    }
  }
  const host = createWorkerHost(endpoint, () => Promise.resolve(engine), {
    pixelBudget: 1,
    createYielder,
  })
  return { endpoint, host, counter }
}

async function importOnHost(endpoint: ControlledEndpoint): Promise<void> {
  endpoint.emitMessage({
    type: 'import',
    requestId: 1,
    operationId: 1,
    name: 'source',
    width: 2,
    height: 1,
    pixels: new ArrayBuffer(8),
  })
  await until(() => endpoint.requests('imported').length === 1, 'import reply')
}

async function exportOnHost(endpoint: ControlledEndpoint, requestId: number): Promise<void> {
  endpoint.emitMessage({
    type: 'export',
    requestId,
    operationId: requestId,
    sourceId: 1,
    parameters: PARAMS,
    outputKind: 'png',
  })
  await until(
    () =>
      endpoint.requests('exported').length +
        endpoint.requests('failed').length +
        endpoint.requests('cancelled').length >
      0,
    'export reply',
  )
}

function transferredChunks(endpoint: ControlledEndpoint) {
  return endpoint.posted.filter(
    ({ message }) => (message as { type?: unknown }).type === 'exportChunk',
  )
}

function concatenate(parts: readonly ArrayBuffer[]): Uint8Array {
  return Uint8Array.from(parts.flatMap((part) => [...new Uint8Array(part)]))
}

describe('worker export transfer', () => {
  it('posts ordered chunks that transfer their own buffers and reports matching totals', async () => {
    const engine = new FakeEngine({ steps: 3 })
    const { endpoint } = hostFixture(engine)
    await importOnHost(endpoint)
    await exportOnHost(endpoint, 2)

    const chunks = transferredChunks(endpoint)
    expect(chunks.map(({ message }) => (message as { sequence: number }).sequence)).toEqual([
      0, 1, 2,
    ])
    chunks.forEach(({ message, transfer }, index) => {
      const bytes = (message as { bytes: ArrayBuffer }).bytes
      expect(transfer).toHaveLength(1)
      expect(transfer[0]).toBe(bytes)
      expect(bytes).toBe(engine.drainedBuffers[index])
    })
    expect(
      concatenate(chunks.map(({ message }) => (message as { bytes: ArrayBuffer }).bytes)),
    ).toEqual(fakePngBytes(3))
    expect(endpoint.requests('exported')).toEqual([
      expect.objectContaining({
        chunkCount: 3,
        byteLength: fakePngBytes(3).byteLength,
      }),
    ])
  })

  it('copies a chunk that spans only part of its buffer into an exact buffer', async () => {
    const engine = new FakeEngine({ steps: 3, subViewChunks: true })
    const { endpoint } = hostFixture(engine)
    await importOnHost(endpoint)
    await exportOnHost(endpoint, 2)

    const chunks = transferredChunks(endpoint)
    const [first, second] = chunks.map(({ message }) => (message as { bytes: ArrayBuffer }).bytes)
    expect(first?.byteLength).toBe(fakeChunkBytes(2).byteLength)
    expect(second?.byteLength).toBe(fakeChunkBytes(3).byteLength)
    expect(first).not.toBe(engine.drainedBuffers[0])
    expect(second).not.toBe(engine.drainedBuffers[1])
    chunks.forEach(({ message, transfer }) => {
      expect(transfer[0]).toBe((message as { bytes: ArrayBuffer }).bytes)
    })
    expect(
      concatenate(chunks.map(({ message }) => (message as { bytes: ArrayBuffer }).bytes)),
    ).toEqual(fakePngBytes(3))
  })

  it('detaches the sender copies when a real MessageChannel transfers them', async () => {
    const engine = new FakeEngine({ steps: 3 })
    const pair = createLinkedPair(engine)
    const source = await pair.client.importRgba(rgba(), 1)
    const exported = await pair.client.exportFile(
      source,
      PARAMS,
      'png',
      2,
      { accept: () => undefined, progress: () => undefined },
      (completion) => ({ ...completion, dispose: () => undefined }),
    )
    expect(exported.chunkCount).toBe(3)
    expect(engine.drainedBuffers.length).toBe(3)
    expect(engine.drainedBuffers.every((buffer) => buffer.byteLength === 0)).toBe(true)
    pair.close()
  })

  it('passes the text output kind to the engine and builds a text Blob over the same grid', async () => {
    const engine = new FakeEngine({ steps: 3 })
    const pair = createLinkedPair(engine)
    const source = await pair.client.importRgba(rgba(), 1)
    const accumulator = createExportAccumulator({ format: 'text' })
    const exported = await pair.client.exportFile(
      source,
      PARAMS,
      'text',
      2,
      accumulator,
      (completion) => accumulator.finish(completion),
    )
    expect(engine.starts.at(-1)).toMatchObject({ kind: 'export', outputKind: 'text' })
    expect(exported.format).toBe('text')
    expect(exported.blob.type).toBe('text/plain;charset=utf-8')
    expect(exported.byteLength).toBe(fakePngBytes(3).byteLength)
    pair.close()
  })

  it('encodes a GIF over every frame with no client frame and monotonic progress in [0, 1]', async () => {
    const engine = new FakeEngine({ steps: 3 })
    engine.animation = { frameCount: 3, frameDelays: [5, 5, 5], loopCount: 0 }
    const pair = createLinkedPair(engine)
    const source = await pair.client.importRgba(rgba(), 1)
    const fractions: number[] = []
    const accumulator = createExportAccumulator({
      format: 'gif',
      onProgress: (fraction) => fractions.push(fraction),
    })
    const exported = await pair.client.exportFile(
      source,
      PARAMS,
      'gif',
      2,
      accumulator,
      (completion) => accumulator.finish(completion),
    )
    expect(engine.starts.at(-1)).toMatchObject({ kind: 'export', outputKind: 'gif', frame: 0 })
    expect(exported.blob.type).toBe('image/gif')
    expect(fractions.length).toBeGreaterThan(0)
    expect(fractions.every((fraction) => fraction >= 0 && fraction <= 1)).toBe(true)
    expect(fractions).toEqual([...fractions].sort((a, b) => a - b))
    pair.close()
  })

  it('yields before every advance so queued messages can run between advances', async () => {
    const engine = new FakeEngine({ steps: 3 })
    const { endpoint, counter } = hostFixture(engine)
    await importOnHost(endpoint)
    const before = counter.yields
    await exportOnHost(endpoint, 2)
    expect(engine.sessions[0]?.advances).toBe(3)
    expect(counter.yields - before).toBe(4)
  })

  it('stops a job when a cancel message arrives between advances', async () => {
    const engine = new FakeEngine({ steps: 10 })
    const { endpoint } = hostFixture(engine)
    await importOnHost(endpoint)
    engine.onAdvance = (advance) => {
      if (advance === 2) endpoint.emitMessage({ type: 'cancel', requestId: 2, operationId: 2 })
    }
    endpoint.emitMessage({
      type: 'export',
      requestId: 2,
      operationId: 2,
      sourceId: 1,
      parameters: PARAMS,
      outputKind: 'png',
    })
    await until(() => endpoint.requests('cancelled').length === 1, 'cancel reply')

    expect(engine.sessions[0]?.advances).toBe(2)
    expect(endpoint.requests('exportChunk')).toHaveLength(0)
    expect(endpoint.requests('exported')).toHaveLength(0)
    expect(engine.log).toContain('cancel:1')
  })

  it('reports an encoder failure as a typed reply and releases the WASM job', async () => {
    const engine = new FakeEngine({ steps: 3, failExportAt: 2 })
    const { endpoint } = hostFixture(engine)
    await importOnHost(endpoint)
    await exportOnHost(endpoint, 2)

    expect(endpoint.requests('failed')).toEqual([
      expect.objectContaining({ code: 'encode_failed', requestId: 2, operationId: 2 }),
    ])
    expect(engine.log).toContain('cancel:1')
  })

  it('rejects an export of a source the worker does not retain', async () => {
    const engine = new FakeEngine({ steps: 3 })
    const { endpoint } = hostFixture(engine)
    endpoint.emitMessage({
      type: 'export',
      requestId: 2,
      operationId: 2,
      sourceId: 9,
      parameters: PARAMS,
      outputKind: 'png',
    })
    await until(() => endpoint.requests('failed').length === 1, 'failure reply')
    expect(endpoint.requests('failed')).toEqual([
      expect.objectContaining({ code: 'unknown_source' }),
    ])
  })
})
