import { describe, expect, it } from 'vitest'
import { createBrowserProcessingAdapter, ExportUnavailableError } from './browserAdapter'
import type { BrowserAdapterOptions } from './browserAdapter'
import type { JobContext, ProcessingAdapter } from './types'
import { DEFAULT_SETTINGS } from './types'
import type {
  PreviewPixels,
  ProcessingClient,
  Releasable,
  RgbaImportSource,
  SourceHandle,
} from './workerClient'

const context = (operationId: number): JobContext => ({ operationId, generation: operationId })

class FakeClient implements ProcessingClient {
  imported: Uint8Array | null = null
  handleDisposals = 0
  clientDisposals = 0
  cancelled: number[] = []
  previews = 0

  async importRgba(source: RgbaImportSource): Promise<SourceHandle> {
    const image = typeof source === 'function' ? await source() : source
    this.imported = new Uint8Array(image.pixels)
    let released = false
    return {
      sourceId: 1,
      name: image.name,
      width: image.width,
      height: image.height,
      dispose: () => {
        if (!released) {
          released = true
          this.handleDisposals += 1
        }
      },
    }
  }

  importEncoded(): Promise<SourceHandle> {
    return Promise.reject(new Error('Encoded import is not used by this test.'))
  }

  originalPreview<T extends Releasable>(
    _source: SourceHandle,
    _operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
  ): Promise<T> {
    return finalize({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  originalPreviewPixels(): Promise<PreviewPixels> {
    return Promise.resolve({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  preview<T extends Releasable>(
    _source: SourceHandle,
    _parameters: unknown,
    _operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
  ): Promise<T> {
    this.previews += 1
    return finalize({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  previewPixels(): Promise<PreviewPixels> {
    return Promise.resolve({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  cancel(operationId: number): void {
    this.cancelled.push(operationId)
  }

  dispose(): void {
    this.clientDisposals += 1
  }
}

function harness(overrides: Partial<BrowserAdapterOptions> = {}): {
  readonly adapter: ProcessingAdapter
  readonly clients: FakeClient[]
  readonly urls: { readonly created: string[]; readonly revoked: string[] }
} {
  const clients: FakeClient[] = []
  const created: string[] = []
  const revoked: string[] = []
  const adapter = createBrowserProcessingAdapter({
    createClient: () => {
      const client = new FakeClient()
      clients.push(client)
      return client
    },
    renderPreview: () => Promise.resolve(new Blob(['png'])),
    urlApi: {
      createObjectURL: () => {
        const url = `blob:browser-${created.length + 1}`
        created.push(url)
        return url
      },
      revokeObjectURL: (url) => revoked.push(url),
    },
    ...overrides,
  })
  return { adapter, clients, urls: { created, revoked } }
}

describe('browser processing adapter', () => {
  it('allocates nothing until the first import', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    expect(clients).toHaveLength(0)
    await adapter.import(session, { kind: 'demo' }, context(1))
    expect(clients).toHaveLength(1)
  })

  it('imports the nonuniform demo fixture through the client', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    const result = await adapter.import(session, { kind: 'demo' }, context(1))
    expect(result.metadata).toEqual({
      name: 'Demo fixture',
      width: 256,
      height: 192,
      source: 'demo',
    })
    const pixels = clients[0]?.imported
    expect(pixels?.byteLength).toBe(256 * 192 * 4)
    expect(new Set(pixels).size).toBeGreaterThan(4)
  })

  it('rejects file input without creating a client', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    await expect(adapter.import(session, {} as File, context(1))).rejects.toThrow('File conversion')
    expect(clients).toHaveLength(0)
  })

  it('revokes a preview URL exactly once and releases failed renders', async () => {
    const { adapter, urls } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    const artifact = await adapter.preview(session, source, DEFAULT_SETTINGS, context(2))
    expect(artifact).toMatchObject({ url: 'blob:browser-1', width: 2, height: 1 })
    artifact.dispose()
    artifact.dispose()
    expect(urls.revoked).toEqual(['blob:browser-1'])

    const failing = harness({ renderPreview: () => Promise.reject(new Error('encode failed')) })
    const failingSession = failing.adapter.createSession()
    const imported = await failing.adapter.import(failingSession, { kind: 'demo' }, context(1))
    await expect(
      failing.adapter.preview(failingSession, imported.source, DEFAULT_SETTINGS, context(2)),
    ).rejects.toThrow('encode failed')
    expect(failing.urls.created).toEqual([])
  })

  it('releases an imported source once and then rejects it', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    const result = await adapter.import(session, { kind: 'demo' }, context(1))
    result.dispose()
    result.dispose()
    expect(clients[0]?.handleDisposals).toBe(1)
    await expect(
      adapter.preview(session, result.source, DEFAULT_SETTINGS, context(2)),
    ).rejects.toThrow('does not belong')
  })

  it('rejects sources from another session or adapter', async () => {
    const first = harness()
    const second = harness()
    const sessionA = first.adapter.createSession()
    const sessionB = first.adapter.createSession()
    const foreign = second.adapter.createSession()
    const { source } = await first.adapter.import(sessionA, { kind: 'demo' }, context(1))
    await expect(
      first.adapter.preview(sessionB, source, DEFAULT_SETTINGS, context(2)),
    ).rejects.toThrow('does not belong')
    await expect(
      second.adapter.preview(foreign, source, DEFAULT_SETTINGS, context(3)),
    ).rejects.toThrow('does not belong')
  })

  it('rejects export explicitly and never fabricates output', async () => {
    const { adapter } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    await expect(
      adapter.export(session, source, 'text', DEFAULT_SETTINGS, context(2)),
    ).rejects.toBeInstanceOf(ExportUnavailableError)
    await expect(
      adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(3)),
    ).rejects.toBeInstanceOf(ExportUnavailableError)
  })

  it('reports a disposed session before export unavailability', async () => {
    const { adapter } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    adapter.dispose(session)
    await expect(
      adapter.export(session, source, 'text', DEFAULT_SETTINGS, context(2)),
    ).rejects.toThrow('disposed')
  })

  it('forwards cancellation and disposes the client once', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    await adapter.import(session, { kind: 'demo' }, context(1))
    adapter.cancel(session, 7)
    adapter.dispose(session)
    adapter.dispose(session)
    adapter.cancel(session, 8)
    expect(clients[0]?.cancelled).toEqual([7])
    expect(clients[0]?.clientDisposals).toBe(1)
    await expect(adapter.import(session, { kind: 'demo' }, context(2))).rejects.toThrow('disposed')
  })
})
