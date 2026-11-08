import { describe, expect, it } from 'vitest'
import { createBrowserProcessingAdapter } from './browserAdapter'
import type { BrowserAdapterOptions } from './browserAdapter'
import { createDemoProcessingAdapter, UnsupportedPreviewKindError } from './demoAdapter'
import type { ExportFormat, JobContext, PreviewKind, ProcessingAdapter } from './types'
import type { OutputKindName, PreviewParameters } from './protocol'
import { DEFAULT_SETTINGS } from './types'
import type { StudioSettings } from './types'
import type {
  EncodedImportSource,
  ExportCompletion,
  ExportSink,
  PreviewPixels,
  ProcessingClient,
  Releasable,
  RgbaImportSource,
  SourceHandle,
} from './workerClient'
import { DEFAULT_RAMP } from './protocol'

const context = (operationId: number): JobContext => ({ operationId, generation: operationId })

class FakeClient implements ProcessingClient {
  imported: Uint8Array | null = null
  encoded: { readonly name: string; readonly bytes: Uint8Array } | null = null
  gate: Promise<void> | null = null
  handleDisposals = 0
  clientDisposals = 0
  cancelled: number[] = []
  previews = 0
  originals = 0
  readonly previewParameters: unknown[] = []
  readonly exportParameters: unknown[] = []
  readonly exportKinds: OutputKindName[] = []

  private handleFor(name: string, width: number, height: number): SourceHandle {
    let released = false
    return {
      sourceId: 1,
      name,
      width,
      height,
      frameCount: 1,
      frameDelays: [],
      loopCount: 0,
      dispose: () => {
        if (!released) {
          released = true
          this.handleDisposals += 1
        }
      },
    }
  }

  async importRgba(source: RgbaImportSource): Promise<SourceHandle> {
    const image = typeof source === 'function' ? await source() : source
    this.imported = new Uint8Array(image.pixels)
    return this.handleFor(image.name, image.width, image.height)
  }

  async importEncoded(source: EncodedImportSource): Promise<SourceHandle> {
    if (this.gate !== null) await this.gate
    const image = typeof source === 'function' ? await source() : source
    this.encoded = { name: image.name, bytes: new Uint8Array(image.bytes) }
    return this.handleFor(image.name, 3, 2)
  }

  exportFile<T extends Releasable>(
    _source: SourceHandle,
    parameters: PreviewParameters,
    outputKind: OutputKindName,
    _operationId: number,
    _sink: ExportSink,
    _finalize: (completion: ExportCompletion) => T,
  ): Promise<T> {
    this.exportParameters.push(parameters)
    this.exportKinds.push(outputKind)
    return Promise.reject(new Error('export is not faked in this suite'))
  }

  originalPreview<T extends Releasable>(
    _source: SourceHandle,
    _operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
  ): Promise<T> {
    this.originals += 1
    return finalize({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  originalPreviewPixels(): Promise<PreviewPixels> {
    return Promise.resolve({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  preview<T extends Releasable>(
    _source: SourceHandle,
    parameters: unknown,
    _operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
  ): Promise<T> {
    this.previews += 1
    this.previewParameters.push(parameters)
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

interface FileProbe {
  readonly file: File
  readonly probe: { reads: number }
}

function fileProbe(
  name: string,
  bytes: readonly number[],
  options: { readonly size?: number; readonly type?: string } = {},
): FileProbe {
  const probe = { reads: 0 }
  const file = new File([new Uint8Array(bytes)], name, { type: options.type ?? '' })
  Object.defineProperty(file, 'arrayBuffer', {
    value: () => {
      probe.reads += 1
      return Promise.resolve(new Uint8Array(bytes).buffer)
    },
  })
  if (options.size !== undefined) {
    Object.defineProperty(file, 'size', { value: options.size })
  }
  return { file, probe }
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

  it('imports a real file through a lazy producer with normalized metadata', async () => {
    let open: () => void = () => undefined
    const client = new FakeClient()
    client.gate = new Promise<void>((resolve) => {
      open = resolve
    })
    const { adapter } = harness({ createClient: () => client })
    const { file, probe } = fileProbe('TIFF.data', [1, 2, 3, 4], { type: 'image/tiff' })
    const session = adapter.createSession()
    const pending = adapter.import(session, file, context(1))
    await Promise.resolve()
    expect(probe.reads).toBe(0)
    open()
    const result = await pending
    expect(probe.reads).toBe(1)
    expect(result.metadata).toEqual({
      name: 'TIFF.data',
      width: 3,
      height: 2,
      source: 'file',
      inputBytes: 4,
    })
    expect(client.encoded).toEqual({ name: 'TIFF.data', bytes: new Uint8Array([1, 2, 3, 4]) })
    expect(client.previews + client.originals).toBe(0)
  })

  it('validates file name and size before creating a client or reading bytes', async () => {
    const cases = [
      fileProbe('empty.png', []),
      fileProbe('huge.png', [1], { size: 256 * 1024 * 1024 + 1 }),
      fileProbe('', [1]),
      fileProbe('n'.repeat(257), [1]),
    ]
    for (const { file, probe } of cases) {
      const { adapter, clients } = harness()
      const session = adapter.createSession()
      await expect(adapter.import(session, file, context(1))).rejects.toBeInstanceOf(RangeError)
      expect(clients).toHaveLength(0)
      expect(probe.reads).toBe(0)
    }
  })

  it('frees the handle and rejects when the session is disposed during a file import', async () => {
    let open: () => void = () => undefined
    const client = new FakeClient()
    client.gate = new Promise<void>((resolve) => {
      open = resolve
    })
    const { adapter } = harness({ createClient: () => client })
    const { file } = fileProbe('late.png', [1, 2])
    const session = adapter.createSession()
    const pending = adapter.import(session, file, context(1))
    const assertion = expect(pending).rejects.toThrow('disposed')
    adapter.dispose(session)
    open()
    await assertion
    expect(client.handleDisposals).toBe(1)
  })

  it('forwards the render mode, ramp and the mode-forced sampling to the client', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    const binary = await adapter.preview(session, source, DEFAULT_SETTINGS, context(2))
    binary.dispose()
    const ascii: StudioSettings = { ...DEFAULT_SETTINGS, mode: 'ascii', ramp: ' .#@' }
    const artwork = await adapter.preview(session, source, ascii, context(3))
    artwork.dispose()
    const block: StudioSettings = { ...DEFAULT_SETTINGS, mode: 'block' }
    const blockArtwork = await adapter.preview(session, source, block, context(4))
    blockArtwork.dispose()
    expect(clients[0]?.previewParameters).toEqual([
      {
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
      {
        cellSize: 12,
        fontSize: 16,
        threshold: 128,
        sampling: 'average',
        mode: 'ascii',
        ramp: ' .#@',
        foreground: '#00ff00',
        background: '#000000',
        colorMode: 'monochrome',
        contrast: 0,
        invert: false,
      },
      {
        cellSize: 12,
        fontSize: 16,
        threshold: 128,
        sampling: 'average',
        mode: 'block',
        ramp: DEFAULT_RAMP,
        foreground: '#00ff00',
        background: '#000000',
        colorMode: 'monochrome',
        contrast: 0,
        invert: false,
      },
    ])
  })

  it('sends the same style snapshot to preview and export', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    const styled: StudioSettings = {
      ...DEFAULT_SETTINGS,
      foreground: '#112233',
      background: '#f0e0d0',
      colorMode: 'source',
      contrast: -25,
      invert: true,
    }
    const artwork = await adapter.preview(session, source, styled, context(2))
    artwork.dispose()
    await expect(adapter.export(session, source, 'png', styled, context(3))).rejects.toThrow(
      'export is not faked',
    )
    const styleOf = (parameters: unknown) => {
      const { foreground, background, colorMode, contrast, invert } = parameters as Record<
        string,
        unknown
      >
      return { foreground, background, colorMode, contrast, invert }
    }
    const expected = {
      foreground: '#112233',
      background: '#f0e0d0',
      colorMode: 'source',
      contrast: -25,
      invert: true,
    }
    expect(styleOf(clients[0]?.previewParameters.at(-1))).toEqual(expected)
    expect(styleOf(clients[0]?.exportParameters.at(-1))).toEqual(expected)
  })

  it('refuses an invalid ramp before any client call, keeping the source usable', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    const bad: StudioSettings = { ...DEFAULT_SETTINGS, mode: 'ascii', ramp: '' }
    await expect(adapter.preview(session, source, bad, context(2))).rejects.toThrow(
      'ramp must contain at least one character',
    )
    await expect(adapter.export(session, source, 'png', bad, context(3))).rejects.toThrow(
      'ramp must contain at least one character',
    )
    expect(clients[0]?.previews).toBe(0)
    expect(clients[0]?.exportParameters).toEqual([])
    const ok = await adapter.preview(session, source, DEFAULT_SETTINGS, context(4))
    ok.dispose()
    expect(clients[0]?.previews).toBe(1)
  })

  it('dispatches original previews for file and demo sources without artwork settings', async () => {
    const { adapter, clients, urls } = harness()
    const session = adapter.createSession()
    const demo = await adapter.import(session, { kind: 'demo' }, context(1))
    const { file } = fileProbe('photo.png', [1, 2, 3])
    const imported = await adapter.import(session, file, context(2))
    const invalid: StudioSettings = {
      cellSize: 1,
      fontSize: 1,
      threshold: 999,
      mode: 'binary' as const,
      ramp: DEFAULT_RAMP,
      foreground: '#00ff00',
      background: '#000000',
      colorMode: 'monochrome',
      contrast: 0,
      invert: false,
    }
    for (const [index, source] of [demo.source, imported.source].entries()) {
      const artifact = await adapter.preview(
        session,
        source,
        invalid,
        context(3 + index),
        'original',
      )
      expect(artifact).toMatchObject({ width: 2, height: 1 })
      artifact.dispose()
      artifact.dispose()
    }
    expect(clients[0]?.originals).toBe(2)
    expect(clients[0]?.previews).toBe(0)
    expect(urls.revoked).toEqual(urls.created)
    expect(urls.created).toHaveLength(2)
    await expect(adapter.preview(session, demo.source, invalid, context(9))).rejects.toBeInstanceOf(
      RangeError,
    )
    await expect(
      adapter.preview(session, demo.source, DEFAULT_SETTINGS, context(10), 'bogus' as PreviewKind),
    ).rejects.toBeInstanceOf(RangeError)
  })

  it('guards original previews against foreign, released, and disposed state', async () => {
    const first = harness()
    const second = harness()
    const session = first.adapter.createSession()
    const foreign = second.adapter.createSession()
    const result = await first.adapter.import(session, { kind: 'demo' }, context(1))
    await expect(
      second.adapter.preview(foreign, result.source, DEFAULT_SETTINGS, context(2), 'original'),
    ).rejects.toThrow('does not belong')
    result.dispose()
    await expect(
      first.adapter.preview(session, result.source, DEFAULT_SETTINGS, context(3), 'original'),
    ).rejects.toThrow('does not belong')
    first.adapter.dispose(session)
    await expect(
      first.adapter.preview(session, result.source, DEFAULT_SETTINGS, context(4), 'original'),
    ).rejects.toThrow('disposed')
  })

  it('makes the demo-only adapter fail explicitly for original previews', async () => {
    const adapter = createDemoProcessingAdapter()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    await expect(
      adapter.preview(session, source, DEFAULT_SETTINGS, context(2), 'original'),
    ).rejects.toBeInstanceOf(UnsupportedPreviewKindError)
    await expect(
      adapter.preview(session, source, DEFAULT_SETTINGS, context(3), 'bogus' as PreviewKind),
    ).rejects.toBeInstanceOf(RangeError)
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

  it('routes PNG and text exports through the client with their output kind', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    await expect(
      adapter.export(session, source, 'png', DEFAULT_SETTINGS, context(2)),
    ).rejects.toThrow('export is not faked')
    await expect(
      adapter.export(session, source, 'text', DEFAULT_SETTINGS, context(3)),
    ).rejects.toThrow('export is not faked')
    expect(clients[0]?.exportKinds).toEqual(['png', 'text'])
  })

  it('rejects an unknown export format before any client call', async () => {
    const { adapter, clients } = harness()
    const session = adapter.createSession()
    const { source } = await adapter.import(session, { kind: 'demo' }, context(1))
    await expect(
      adapter.export(
        session,
        source,
        'svg' as unknown as ExportFormat,
        DEFAULT_SETTINGS,
        context(2),
      ),
    ).rejects.toBeInstanceOf(RangeError)
    expect(clients[0]?.exportKinds).toEqual([])
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
