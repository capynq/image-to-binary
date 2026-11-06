import { describe, expect, it } from 'vitest'
import { createAdapterWithAccumulator } from './browserAdapter'
import type { BrowserAdapterOptions } from './browserAdapter'
import { createExportAccumulator, ExportLimitError } from './exportAccumulator'
import type { ExportAccumulator, ExportAccumulatorOptions } from './exportAccumulator'
import { createLinkedPair, FakeEngine, fakePngBytes, rgba, settle, until } from './testKit'
import type { LinkedPair } from './testKit'
import type { JobContext, ProcessingAdapter, ProcessingSession } from './types'
import { DEFAULT_SETTINGS } from './types'

interface Observed {
  accepted: number
  released: number
}

const context = (operationId: number): JobContext => ({ operationId, generation: operationId })

/** Wraps the real accumulator so tests can count accepts and releases and react to them. */
function observedAccumulators(
  onAccept: (record: Observed, index: number) => void = () => undefined,
  limitBytes?: number,
) {
  const records: Observed[] = []
  const create = (options: ExportAccumulatorOptions): ExportAccumulator => {
    const real = createExportAccumulator({
      ...options,
      ...(limitBytes === undefined ? {} : { limitBytes }),
    })
    const record: Observed = { accepted: 0, released: 0 }
    records.push(record)
    const index = records.length - 1
    return {
      accept: (chunk) => {
        record.accepted += 1
        real.accept(chunk)
        onAccept(record, index)
      },
      progress: (fraction) => real.progress(fraction),
      finish: (completion) => real.finish(completion),
      release: () => {
        record.released += 1
        real.release()
      },
    }
  }
  return { records, create }
}

function adapterOver(
  pair: LinkedPair,
  makeAccumulator: (options: ExportAccumulatorOptions) => ExportAccumulator,
): ProcessingAdapter {
  const options: BrowserAdapterOptions = {
    createClient: () => pair.client,
    createDemoImage: () => rgba(),
  }
  return createAdapterWithAccumulator(options, makeAccumulator)
}

async function openSource(adapter: ProcessingAdapter) {
  const session: ProcessingSession = adapter.createSession()
  const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
  return { session, source }
}

describe('export chunk release on terminal paths', () => {
  it('a throwing progress listener fails the export, releases chunks and leaves the adapter usable', async () => {
    const pair = createLinkedPair(new FakeEngine({ steps: 3 }))
    const { records, create } = observedAccumulators()
    const adapter = adapterOver(pair, create)
    const { session, source } = await openSource(adapter)

    const failed = await settle(
      adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(2), () => {
        throw new Error('progress listener failed')
      }),
    )
    expect(failed.ok).toBe(false)
    if (!failed.ok) expect(failed.error.message).toBe('progress listener failed')
    expect(records[0]?.released).toBe(1)

    const artifact = await adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(3))
    expect(new Uint8Array(await artifact.blob.arrayBuffer())).toEqual(fakePngBytes(3))
    expect(records[1]?.released).toBe(1)
    pair.close()
  })

  it('adapter cancel after chunks were accumulated releases them and publishes nothing', async () => {
    const pair = createLinkedPair(new FakeEngine({ steps: 3 }))
    let adapterRef: ProcessingAdapter | null = null
    let sessionRef: ProcessingSession | null = null
    const { records, create } = observedAccumulators((record) => {
      if (record.accepted === 1) adapterRef?.cancel(sessionRef as ProcessingSession, 2)
    })
    const adapter = adapterOver(pair, create)
    adapterRef = adapter
    const { session, source } = await openSource(adapter)
    sessionRef = session

    const outcome = await settle(
      adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(2)),
    )
    expect(outcome.ok).toBe(false)
    expect(records[0]?.accepted).toBe(1)
    expect(records[0]?.released).toBe(1)
    pair.close()
  })

  it('disposing the session during export releases accumulated chunks and publishes nothing', async () => {
    const pair = createLinkedPair(new FakeEngine({ steps: 3 }))
    let adapterRef: ProcessingAdapter | null = null
    let sessionRef: ProcessingSession | null = null
    const { records, create } = observedAccumulators((record) => {
      if (record.accepted === 1) adapterRef?.dispose(sessionRef as ProcessingSession)
    })
    const adapter = adapterOver(pair, create)
    adapterRef = adapter
    const { session, source } = await openSource(adapter)
    sessionRef = session

    const outcome = await settle(
      adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(2)),
    )
    expect(outcome.ok).toBe(false)
    expect(records[0]?.accepted).toBe(1)
    expect(records[0]?.released).toBe(1)
    pair.close()
  })

  it('a sink failure at the cumulative limit releases accumulated chunks and rejects with a typed error', async () => {
    const pair = createLinkedPair(new FakeEngine({ steps: 3 }))
    const { records, create } = observedAccumulators(() => undefined, 3)
    const adapter = adapterOver(pair, create)
    const { session, source } = await openSource(adapter)

    const outcome = await settle(
      adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(2)),
    )
    expect(outcome.ok).toBe(false)
    if (!outcome.ok) expect(outcome.error).toBeInstanceOf(ExportLimitError)
    expect(records[0]?.accepted).toBe(2)
    expect(records[0]?.released).toBe(1)
    await until(() => (records[0]?.released ?? 0) === 1, 'release')
    pair.close()
  })
})
