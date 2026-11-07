import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import type { JobContext, ProcessingAdapter, StudioMetadata } from '../processing/types'
import { DEFAULT_SETTINGS } from '../processing/types'
import { createStudioController, StaleOperationError } from './controller'

const context = (operationId: number): JobContext => ({ operationId, generation: operationId })

function deferred<T>(): { readonly promise: Promise<T>; readonly resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((next) => {
    resolve = next
  })
  return { promise, resolve }
}

function urlApi(): {
  readonly created: string[]
  readonly revoked: string[]
  createObjectURL(): string
  revokeObjectURL(url: string): void
} {
  const created: string[] = []
  const revoked: string[] = []
  return {
    created,
    revoked,
    createObjectURL: () => {
      const url = `blob:acceptance-${created.length + 1}`
      created.push(url)
      return url
    },
    revokeObjectURL: (url) => revoked.push(url),
  }
}

function wrapperAdapter(
  options: {
    readonly beforeImport?: (context: JobContext) => Promise<void>
    readonly beforeExport?: (context: JobContext) => Promise<void>
  } = {},
): ProcessingAdapter {
  const base = createDemoProcessingAdapter()
  return {
    createSession: () => base.createSession(),
    import: async (session, input, operation) => {
      await options.beforeImport?.(operation)
      const result = await base.import(session, input, operation)
      return {
        ...result,
        metadata: { ...result.metadata, name: `Demo fixture ${operation.operationId}` },
      }
    },
    preview: (session, source, settings, operation) =>
      base.preview(session, source, settings, operation),
    export: async (session, source, format, settings, operation) => {
      if (options.beforeExport !== undefined) {
        await options.beforeExport(operation)
        const blob = new Blob(['accepted stale output'], { type: 'text/plain' })
        return { kind: 'blob', blob, byteLength: blob.size, dispose: () => undefined }
      }
      return base.export(session, source, format, settings, operation)
    },
    cancel: (session, operationId) => base.cancel(session, operationId),
    dispose: (session) => base.dispose(session),
  }
}

describe('phase 3.1 acceptance contracts', () => {
  it('keeps the newer import when an older import resolves later', async () => {
    const gates: Array<{ readonly promise: Promise<void>; readonly resolve: () => void }> = []
    const adapter = wrapperAdapter({
      beforeImport: async () => {
        const gate = deferred<void>()
        gates.push({ promise: gate.promise, resolve: () => gate.resolve(undefined) })
        await gate.promise
      },
    })
    const controller = createStudioController({ adapter })
    const older = controller.loadDemo()
    const newer = controller.loadDemo()
    expect(gates).toHaveLength(2)
    const olderGate = gates[0]
    const newerGate = gates[1]
    if (olderGate === undefined || newerGate === undefined)
      throw new Error('Expected two import gates.')
    newerGate.resolve()
    await newer
    olderGate.resolve()
    await older
    expect(controller.getSnapshot().metadata?.name).toBe('Demo fixture 2')
    expect(controller.getSnapshot().status).toBe('ready')
  })

  it('rejects a stale export after cancellation without publishing output', async () => {
    const gate = deferred<void>()
    const controller = createStudioController({
      adapter: wrapperAdapter({ beforeExport: () => gate.promise }),
    })
    await controller.loadDemo()
    const pending = controller.exportDemo('text')
    controller.cancel()
    gate.resolve(undefined)
    await expect(pending).rejects.toBeInstanceOf(StaleOperationError)
    expect(controller.getSnapshot().status).toBe('cancelled')
  })

  it('applies threshold to the shared fixture for preview and text export', async () => {
    const controller = createStudioController()
    await controller.loadDemo()
    controller.updateSettings({ cellSize: 12, fontSize: 16, threshold: 0 })
    await controller.preview()
    expect(await (await controller.exportDemo('text')).text()).toBe('0000\n0000\n0000\n0000')
    controller.updateSettings({ cellSize: 12, fontSize: 16, threshold: 255 })
    await controller.preview()
    expect(await (await controller.exportDemo('text')).text()).toBe('1111\n1111\n1111\n1111')
  })

  it('does not revive the oldest cancelled preview after many newer operations', async () => {
    const urls = urlApi()
    const gates: Array<() => void> = []
    const adapter = createDemoProcessingAdapter({
      urlApi: urls,
      delay: () =>
        new Promise<void>((resolve) => {
          gates.push(resolve)
        }),
    })
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    const previews = Array.from({ length: 33 }, () => controller.preview())
    expect(gates).toHaveLength(33)
    for (const release of gates.slice(1)) release()
    await Promise.all(previews.slice(1))
    gates[0]?.()
    await previews[0]
    expect(urls.created).toHaveLength(1)
    expect(controller.getSnapshot().preview?.url).toBe('blob:acceptance-1')
  })

  it('revokes replaced and disposed previews exactly once', async () => {
    const urls = urlApi()
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({ urlApi: urls }),
    })
    await controller.loadDemo()
    await controller.preview()
    await controller.preview()
    controller.dispose()
    controller.dispose()
    expect(urls.revoked).toEqual(['blob:acceptance-1', 'blob:acceptance-2'])
  })

  it('rejects foreign sessions and sources across adapter boundaries', async () => {
    const first = createDemoProcessingAdapter()
    const second = createDemoProcessingAdapter()
    const firstSession = first.createSession()
    const secondSession = first.createSession()
    const foreignSession = second.createSession()
    const imported = await first.import(firstSession, { kind: 'demo' }, context(1))
    await expect(
      first.preview(secondSession, imported.source, DEFAULT_SETTINGS, context(2)),
    ).rejects.toThrow('does not belong')
    await expect(
      second.preview(foreignSession, imported.source, DEFAULT_SETTINGS, context(3)),
    ).rejects.toThrow('does not belong')
  })

  it.each([
    [{ cellSize: 4, fontSize: 16, threshold: 128 }, 'Cell size'],
    [{ cellSize: 12, fontSize: 7, threshold: 128 }, 'Font size'],
    [{ cellSize: 12, fontSize: 16, threshold: 256 }, 'Threshold'],
  ] as const)('validates %s', (settings, message) => {
    const controller = createStudioController()
    expect(() => controller.updateSettings(settings)).toThrow(message)
  })

  it('freezes nested snapshots and isolates adapter-owned metadata', async () => {
    let adapterMetadata: { -readonly [K in keyof StudioMetadata]: StudioMetadata[K] } | undefined
    const base = createDemoProcessingAdapter()
    const adapter: ProcessingAdapter = {
      createSession: () => base.createSession(),
      import: async (session, input, operation) => {
        const result = await base.import(session, input, operation)
        const ownedMetadata = { ...result.metadata }
        adapterMetadata = ownedMetadata
        return { ...result, metadata: ownedMetadata }
      },
      preview: (session, source, settings, operation) =>
        base.preview(session, source, settings, operation),
      export: (session, source, format, settings, operation) =>
        base.export(session, source, format, settings, operation),
      cancel: (session, operationId) => base.cancel(session, operationId),
      dispose: (session) => base.dispose(session),
    }
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    await controller.preview()
    const before = controller.getSnapshot()
    expect(Object.isFrozen(before)).toBe(true)
    expect(Object.isFrozen(before.settings)).toBe(true)
    expect(Object.isFrozen(before.metadata)).toBe(true)
    expect(Object.isFrozen(before.preview)).toBe(true)
    expect(Reflect.set(before.settings, 'cellSize', 30)).toBe(false)
    expect(Reflect.set(before.metadata!, 'name', 'changed')).toBe(false)
    adapterMetadata!.name = 'adapter mutation'
    expect(controller.getSnapshot().metadata?.name).toBe('Demo fixture')
  })

  it('preserves the previous artifact after a failed replacement import', async () => {
    const urls = urlApi()
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({ urlApi: urls }),
    })
    await controller.loadDemo()
    await controller.preview()
    const previous = controller.getSnapshot().preview
    await expect(
      controller.import(new File(['input'], 'input.png', { type: 'image/png' })),
    ).rejects.toThrow('File conversion')
    expect(controller.getSnapshot().preview).toEqual(previous)
    expect(controller.getSnapshot().error).toContain('Previous image remains available.')
    const exported = await controller.exportDemo('text')
    expect(await exported.text()).toBe('1010\n0101\n1010\n0101')
  })
})
