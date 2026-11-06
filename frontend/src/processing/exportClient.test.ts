import { describe, expect, it } from 'vitest'
import { createBrowserProcessingAdapter } from './browserAdapter'
import { ExportLimitError } from './exportAccumulator'
import {
  ControlledEndpoint,
  createLinkedPair,
  FakeEngine,
  fakePngBytes,
  importViaEndpoint,
  PARAMS,
  rgba,
  settle,
  until,
} from './testKit'
import { createProcessingClient } from './workerClient'
import type { ExportCompletion, ExportSink, ProcessingClient } from './workerClient'
import { DEFAULT_SETTINGS } from './types'
import type { JobContext } from './types'

interface RecordingSink extends ExportSink {
  readonly accepted: ArrayBuffer[]
  readonly fractions: number[]
}

function recordingSink(): RecordingSink {
  const accepted: ArrayBuffer[] = []
  const fractions: number[] = []
  return {
    accepted,
    fractions,
    accept: (chunk) => {
      accepted.push(chunk)
    },
    progress: (fraction) => {
      fractions.push(fraction)
    },
  }
}

const finalize = (completion: ExportCompletion) => ({ ...completion, dispose: () => undefined })

const previewFinalize = async (pixels: { width: number; height: number }) => ({
  width: pixels.width,
  height: pixels.height,
  dispose: () => undefined,
})

function controlledClient(): { endpoint: ControlledEndpoint; client: ProcessingClient } {
  const endpoint = new ControlledEndpoint()
  const client = createProcessingClient({ createEndpoint: () => endpoint })
  return { endpoint, client }
}

function latest(endpoint: ControlledEndpoint, type: string): Record<string, unknown> {
  const request = endpoint.requests(type).at(-1)
  if (request === undefined) throw new Error(`No ${type} message was posted.`)
  return request
}

function emitChunk(
  endpoint: ControlledEndpoint,
  requestId: number,
  operationId: number,
  sequence: number,
  values: number[],
): void {
  endpoint.emitMessage({
    type: 'exportChunk',
    requestId,
    operationId,
    sequence,
    bytes: Uint8Array.from(values).buffer,
  })
}

const context = (operationId: number): JobContext => ({ operationId, generation: operationId })

describe('export client stream', () => {
  it('accepts ordered chunks and resolves with totals that match what it received', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    const sink = recordingSink()
    const pending = client.exportPng(source, PARAMS, 2, sink, finalize)
    await until(() => endpoint.requests('export').length === 1, 'export request')
    const { requestId } = latest(endpoint, 'export')

    emitChunk(endpoint, Number(requestId), 2, 0, [1, 2])
    endpoint.emitMessage({
      type: 'exportProgress',
      requestId,
      operationId: 2,
      completed: 1,
      total: 2,
    })
    emitChunk(endpoint, Number(requestId), 2, 1, [3])
    endpoint.emitMessage({
      type: 'exported',
      requestId,
      operationId: 2,
      chunkCount: 2,
      byteLength: 3,
    })

    expect(await pending).toEqual({ chunkCount: 2, byteLength: 3, dispose: expect.any(Function) })
    expect(sink.accepted.map((chunk) => [...new Uint8Array(chunk)])).toEqual([[1, 2], [3]])
    expect(sink.fractions).toEqual([0.5])
  })

  it('rejects out-of-order chunks and fails the session', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    const sink = recordingSink()
    const pending = client.exportPng(source, PARAMS, 2, sink, finalize)
    await until(() => endpoint.requests('export').length === 1, 'export request')
    emitChunk(endpoint, Number(latest(endpoint, 'export').requestId), 2, 1, [9])

    const outcome = await settle(pending)
    expect(outcome.ok).toBe(false)
    expect(sink.accepted).toHaveLength(0)
    if (!outcome.ok) expect(outcome.error.message).toContain('out of order')
  })

  it('drops chunks and progress whose operation identity is not the current export', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    const sink = recordingSink()
    const pending = client.exportPng(source, PARAMS, 2, sink, finalize)
    await until(() => endpoint.requests('export').length === 1, 'export request')
    const requestId = Number(latest(endpoint, 'export').requestId)

    emitChunk(endpoint, requestId, 77, 0, [5])
    endpoint.emitMessage({
      type: 'exportProgress',
      requestId,
      operationId: 77,
      completed: 1,
      total: 1,
    })
    emitChunk(endpoint, requestId, 2, 0, [1])
    endpoint.emitMessage({
      type: 'exported',
      requestId,
      operationId: 2,
      chunkCount: 1,
      byteLength: 1,
    })

    expect(await pending).toMatchObject({ chunkCount: 1, byteLength: 1 })
    expect(sink.accepted.map((chunk) => [...new Uint8Array(chunk)])).toEqual([[1]])
    expect(sink.fractions).toEqual([])
  })

  it('cancelling an export drops later chunks and progress and rejects promptly', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    const sink = recordingSink()
    const pending = client.exportPng(source, PARAMS, 2, sink, finalize)
    await until(() => endpoint.requests('export').length === 1, 'export request')
    const requestId = Number(latest(endpoint, 'export').requestId)

    client.cancel(2)
    expect(endpoint.requests('cancel')).toEqual([
      expect.objectContaining({ requestId, operationId: 2 }),
    ])
    emitChunk(endpoint, requestId, 2, 0, [9])
    endpoint.emitMessage({
      type: 'exportProgress',
      requestId,
      operationId: 2,
      completed: 1,
      total: 2,
    })
    endpoint.emitMessage({ type: 'cancelled', requestId, operationId: 2 })

    expect((await settle(pending)).ok).toBe(false)
    expect(sink.accepted).toHaveLength(0)
    expect(sink.fractions).toEqual([])
  })

  it('a sink failure cancels the worker job, rejects with its error and accepts nothing more', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    let calls = 0
    const sink: ExportSink = {
      accept: () => {
        calls += 1
        throw new ExportLimitError(1)
      },
      progress: () => undefined,
    }
    const pending = client.exportPng(source, PARAMS, 2, sink, finalize)
    await until(() => endpoint.requests('export').length === 1, 'export request')
    const requestId = Number(latest(endpoint, 'export').requestId)

    emitChunk(endpoint, requestId, 2, 0, [1, 2])
    expect(endpoint.requests('cancel')).toHaveLength(1)
    emitChunk(endpoint, requestId, 2, 1, [3])
    endpoint.emitMessage({ type: 'cancelled', requestId, operationId: 2 })

    const outcome = await settle(pending)
    expect(outcome.ok).toBe(false)
    if (!outcome.ok) expect(outcome.error).toBeInstanceOf(ExportLimitError)
    expect(calls).toBe(1)
  })

  it('a late completion after supersession never finalizes', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    let finalized = 0
    const pending = client.exportPng(source, PARAMS, 2, recordingSink(), (completion) => {
      finalized += 1
      return finalize(completion)
    })
    await until(() => endpoint.requests('export').length === 1, 'export request')
    const exportId = Number(latest(endpoint, 'export').requestId)

    const preview = client.preview(source, PARAMS, 3, previewFinalize)
    expect((await settle(pending)).ok).toBe(false)
    endpoint.emitMessage({
      type: 'exported',
      requestId: exportId,
      operationId: 2,
      chunkCount: 0,
      byteLength: 0,
    })
    await until(() => endpoint.requests('preview').length === 1, 'preview after export unwinds')
    expect(finalized).toBe(0)

    const previewId = Number(latest(endpoint, 'preview').requestId)
    endpoint.emitMessage({
      type: 'previewed',
      requestId: previewId,
      operationId: 3,
      width: 2,
      height: 1,
      pixels: new ArrayBuffer(8),
    })
    expect((await settle(preview)).ok).toBe(true)
  })

  it('a newer preview supersedes a running export, which rejects before the worker unwinds', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    const exportPending = client.exportPng(source, PARAMS, 2, recordingSink(), finalize)
    await until(() => endpoint.requests('export').length === 1, 'export request')
    const exportId = Number(latest(endpoint, 'export').requestId)

    const previewPending = client.preview(source, PARAMS, 3, previewFinalize)
    expect((await settle(exportPending)).ok).toBe(false)
    expect(endpoint.requests('preview')).toHaveLength(0)

    endpoint.emitMessage({ type: 'cancelled', requestId: exportId, operationId: 2 })
    await until(() => endpoint.requests('preview').length === 1, 'preview request')
    const previewId = Number(latest(endpoint, 'preview').requestId)
    endpoint.emitMessage({
      type: 'previewed',
      requestId: previewId,
      operationId: 3,
      width: 2,
      height: 1,
      pixels: new ArrayBuffer(8),
    })
    expect((await settle(previewPending)).ok).toBe(true)
  })

  it('a newer export replaces a pending export, and the preview after it runs in order', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    const activePreview = client.preview(source, PARAMS, 2, previewFinalize)
    await until(() => endpoint.requests('preview').length === 1, 'active preview')
    const activeId = Number(latest(endpoint, 'preview').requestId)

    const pendingExport = client.exportPng(source, PARAMS, 3, recordingSink(), finalize)
    const newerPreview = client.preview(source, PARAMS, 4, previewFinalize)
    expect((await settle(pendingExport)).ok).toBe(false)
    expect(endpoint.requests('export')).toHaveLength(0)

    endpoint.emitMessage({ type: 'cancelled', requestId: activeId, operationId: 2 })
    expect((await settle(activePreview)).ok).toBe(false)
    await until(() => endpoint.requests('preview').length === 2, 'newer preview request')
    const newerId = Number(latest(endpoint, 'preview').requestId)
    endpoint.emitMessage({
      type: 'previewed',
      requestId: newerId,
      operationId: 4,
      width: 2,
      height: 1,
      pixels: new ArrayBuffer(8),
    })
    expect((await settle(newerPreview)).ok).toBe(true)
  })

  it('disposing the client rejects a running export without finalizing', async () => {
    const { endpoint, client } = controlledClient()
    const source = await importViaEndpoint(client, endpoint, 1, 1)
    let finalized = 0
    const pending = client.exportPng(source, PARAMS, 2, recordingSink(), (completion) => {
      finalized += 1
      return finalize(completion)
    })
    await until(() => endpoint.requests('export').length === 1, 'export request')
    client.dispose()
    expect((await settle(pending)).ok).toBe(false)
    expect(finalized).toBe(0)
  })
})

describe('browser adapter PNG export', () => {
  it('publishes an exact image/png Blob of the worker output with monotonic progress', async () => {
    const pair = createLinkedPair(new FakeEngine({ steps: 3 }))
    const adapter = createBrowserProcessingAdapter({
      createClient: () => pair.client,
      createDemoImage: () => rgba(),
    })
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    const fractions: number[] = []
    const artifact = await adapter.export(
      session,
      source,
      'png',
      DEFAULT_SETTINGS,
      context(2),
      (fraction) => fractions.push(fraction),
    )

    expect(artifact.blob.type).toBe('image/png')
    expect(artifact.byteLength).toBe(fakePngBytes(3).byteLength)
    expect(artifact.blob.size).toBe(artifact.byteLength)
    expect(new Uint8Array(await artifact.blob.arrayBuffer())).toEqual(fakePngBytes(3))
    expect(fractions).toEqual([...fractions].sort((left, right) => left - right))
    expect(fractions.at(-1)).toBe(1)
    pair.close()
  })

  it('cancelling an adapter export rejects and yields no artifact', async () => {
    const pair = createLinkedPair(new FakeEngine({ steps: 3 }))
    const adapter = createBrowserProcessingAdapter({
      createClient: () => pair.client,
      createDemoImage: () => rgba(),
    })
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    const pending = adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(2))
    adapter.cancel(session, 2)
    expect((await settle(pending)).ok).toBe(false)
    pair.close()
  })
})
