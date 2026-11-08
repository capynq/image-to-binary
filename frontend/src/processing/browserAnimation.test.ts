import { describe, expect, it } from 'vitest'
import {
  ANIMATION_BUDGET_MESSAGE,
  ANIMATION_PREVIEW_BUDGET_BYTES,
  createBrowserProcessingAdapter,
} from './browserAdapter'
import type { BrowserAdapterOptions } from './browserAdapter'
import type { OutputKindName, PreviewParameters } from './protocol'
import { until } from './testKit'
import type { JobContext, ProcessingAdapter, ProcessingSession, ImportedResult } from './types'
import { DEFAULT_SETTINGS } from './types'
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

const MiB = 1024 * 1024
const context = (operationId: number): JobContext => ({ operationId, generation: operationId })

/** Records the frame index of every frame request and can hold a frame open before it finishes. */
class AnimationClient implements ProcessingClient {
  readonly previewFrames: number[] = []
  readonly originalFrames: number[] = []
  readonly exportFrames: number[] = []
  readonly cancelled: number[] = []
  disposals = 0
  beforeFrame: ((frame: number) => Promise<void> | undefined) | null = null

  constructor(
    private readonly frameCount: number,
    private readonly delays: ReadonlyArray<number> = Array.from({ length: frameCount }, () => 10),
  ) {}

  private handle(name: string, width: number, height: number): SourceHandle {
    const animated = this.frameCount > 1
    return {
      sourceId: 1,
      name,
      width,
      height,
      frameCount: this.frameCount,
      frameDelays: animated ? this.delays : [],
      loopCount: 0,
      dispose: () => undefined,
    }
  }

  async importRgba(source: RgbaImportSource): Promise<SourceHandle> {
    const image = typeof source === 'function' ? await source() : source
    return this.handle(image.name, image.width, image.height)
  }

  async importEncoded(source: EncodedImportSource): Promise<SourceHandle> {
    const image = typeof source === 'function' ? await source() : source
    return this.handle(image.name, 2, 1)
  }

  async preview<T extends Releasable>(
    _source: SourceHandle,
    _parameters: PreviewParameters,
    _operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
    frame = 0,
  ): Promise<T> {
    this.previewFrames.push(frame)
    await this.beforeFrame?.(frame)
    return finalize({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  async originalPreview<T extends Releasable>(
    _source: SourceHandle,
    _operationId: number,
    finalize: (pixels: PreviewPixels) => Promise<T>,
    frame = 0,
  ): Promise<T> {
    this.originalFrames.push(frame)
    await this.beforeFrame?.(frame)
    return finalize({ width: 2, height: 1, pixels: new ArrayBuffer(8) })
  }

  previewPixels(): Promise<PreviewPixels> {
    return Promise.reject(new Error('not used'))
  }

  originalPreviewPixels(): Promise<PreviewPixels> {
    return Promise.reject(new Error('not used'))
  }

  exportFile<T extends Releasable>(
    _source: SourceHandle,
    _parameters: PreviewParameters,
    _outputKind: OutputKindName,
    _operationId: number,
    sink: ExportSink,
    finalize: (completion: ExportCompletion) => T,
    frame = 0,
  ): Promise<T> {
    this.exportFrames.push(frame)
    sink.accept(Uint8Array.of(65).buffer)
    return Promise.resolve(finalize({ chunkCount: 1, byteLength: 1 }))
  }

  cancel(operationId: number): void {
    this.cancelled.push(operationId)
  }

  dispose(): void {
    this.disposals += 1
  }
}

function harness(client: AnimationClient, overrides: Partial<BrowserAdapterOptions> = {}) {
  const created: string[] = []
  const revoked: string[] = []
  const adapter = createBrowserProcessingAdapter({
    createClient: () => client,
    renderPreview: () => Promise.resolve(new Blob(['png'])),
    urlApi: {
      createObjectURL: () => {
        const url = `blob:frame-${created.length + 1}`
        created.push(url)
        return url
      },
      revokeObjectURL: (url) => revoked.push(url),
    },
    ...overrides,
  })
  return { adapter, created, revoked }
}

async function importDemo(adapter: ProcessingAdapter): Promise<{
  readonly session: ProcessingSession
  readonly imported: ImportedResult
}> {
  const session = adapter.createSession()
  const imported = await adapter.import(session, { kind: 'demo' }, context(1))
  return { session, imported }
}

describe('browser adapter animation sources', () => {
  it('exposes the animation of an imported source and null for a still', async () => {
    const animated = harness(new AnimationClient(3, [5, 10, 2]))
    const { imported } = await importDemo(animated.adapter)
    expect(imported.animation).toEqual({ frameCount: 3, delaysCs: [5, 10, 2], loopCount: 0 })
    expect(Object.isFrozen(imported.animation)).toBe(true)

    const still = harness(new AnimationClient(1))
    const stillImport = await importDemo(still.adapter)
    expect(stillImport.imported.animation).toBeNull()
  })

  it('renders every frame from the start frame, wrapping around, with frame-order urls', async () => {
    const client = new AnimationClient(4)
    const { adapter, created } = harness(client)
    const { session, imported } = await importDemo(adapter)
    const progress: number[] = []
    const artifact = await adapter.preview(
      session,
      imported.source,
      DEFAULT_SETTINGS,
      context(2),
      'artwork',
      (fraction) => progress.push(fraction),
      { startFrame: 2 },
    )
    expect(client.previewFrames).toEqual([2, 3, 0, 1])
    expect(created).toEqual(['blob:frame-1', 'blob:frame-2', 'blob:frame-3', 'blob:frame-4'])
    expect(artifact.url).toBe('blob:frame-1')
    expect(artifact.frames).toEqual([
      'blob:frame-3',
      'blob:frame-4',
      'blob:frame-1',
      'blob:frame-2',
    ])
    expect(progress).toEqual([0.25, 0.5, 0.75, 1])
  })

  it('renders the original kind of every frame with the same wrap-around', async () => {
    const client = new AnimationClient(3)
    const { adapter } = harness(client)
    const { session, imported } = await importDemo(adapter)
    const artifact = await adapter.preview(
      session,
      imported.source,
      DEFAULT_SETTINGS,
      context(2),
      'original',
      undefined,
      { startFrame: 1 },
    )
    expect(client.originalFrames).toEqual([1, 2, 0])
    expect(client.previewFrames).toEqual([])
    expect(artifact.frames?.length).toBe(3)
  })

  it('keeps a still preview unchanged, with no frames property', async () => {
    const client = new AnimationClient(1)
    const { adapter, created } = harness(client)
    const { session, imported } = await importDemo(adapter)
    const artifact = await adapter.preview(session, imported.source, DEFAULT_SETTINGS, context(2))
    expect(client.previewFrames).toEqual([0])
    expect(artifact).not.toHaveProperty('frames')
    expect(artifact.url).toBe(created[0])
  })

  it('rejects a start frame outside the source before any frame is rendered', async () => {
    const client = new AnimationClient(4)
    const { adapter } = harness(client)
    const { session, imported } = await importDemo(adapter)
    for (const startFrame of [4, -1, 1.5]) {
      await expect(
        adapter.preview(
          session,
          imported.source,
          DEFAULT_SETTINGS,
          context(2),
          'artwork',
          undefined,
          {
            startFrame,
          },
        ),
      ).rejects.toThrow(RangeError)
    }
    expect(client.previewFrames).toEqual([])

    const still = harness(new AnimationClient(1))
    const stillSession = await importDemo(still.adapter)
    await expect(
      still.adapter.preview(
        stillSession.session,
        stillSession.imported.source,
        DEFAULT_SETTINGS,
        context(2),
        'artwork',
        undefined,
        { startFrame: 1 },
      ),
    ).rejects.toThrow(RangeError)
  })

  it('cancelling between frames stops the run and revokes every frame already made', async () => {
    const client = new AnimationClient(3)
    const gate = { release: (): void => undefined }
    const held = new Promise<void>((resolve) => {
      gate.release = resolve
    })
    client.beforeFrame = (frame) => (frame === 1 ? held : undefined)
    const { adapter, created, revoked } = harness(client)
    const { session, imported } = await importDemo(adapter)
    const run = adapter.preview(session, imported.source, DEFAULT_SETTINGS, context(2))
    await until(() => client.previewFrames.length === 2, 'frame 1 start')
    adapter.cancel(session, 2)
    gate.release()
    await expect(run).rejects.toThrow('Animation preview was cancelled.')
    expect(client.previewFrames).toEqual([0, 1])
    expect(created).toHaveLength(2)
    expect([...revoked].sort()).toEqual([...created].sort())
  })

  it('revokes the frames already made when a later frame fails', async () => {
    const client = new AnimationClient(3)
    client.beforeFrame = (frame) => {
      if (frame === 2) throw new Error('frame 2 failed')
      return undefined
    }
    const { adapter, created, revoked } = harness(client)
    const { session, imported } = await importDemo(adapter)
    await expect(
      adapter.preview(session, imported.source, DEFAULT_SETTINGS, context(2)),
    ).rejects.toThrow('frame 2 failed')
    expect(created).toHaveLength(2)
    expect([...revoked].sort()).toEqual([...created].sort())
  })

  it('accepts exactly the 512 MiB budget and rejects the frame that exceeds it', async () => {
    expect(ANIMATION_PREVIEW_BUDGET_BYTES).toBe(512 * MiB)

    const atBudget = harness(new AnimationClient(2), {
      renderPreview: () => Promise.resolve({ size: 256 * MiB } as Blob),
    })
    const accepted = await importDemo(atBudget.adapter)
    const artifact = await atBudget.adapter.preview(
      accepted.session,
      accepted.imported.source,
      DEFAULT_SETTINGS,
      context(2),
    )
    expect(artifact.frames).toHaveLength(2)

    const overBudget = harness(new AnimationClient(3), {
      renderPreview: () => Promise.resolve({ size: 300 * MiB } as Blob),
    })
    const rejected = await importDemo(overBudget.adapter)
    await expect(
      overBudget.adapter.preview(
        rejected.session,
        rejected.imported.source,
        DEFAULT_SETTINGS,
        context(2),
      ),
    ).rejects.toThrow(ANIMATION_BUDGET_MESSAGE)
    expect(overBudget.created).toEqual(['blob:frame-1'])
    expect(overBudget.revoked).toEqual(['blob:frame-1'])
  })

  it('disposes every frame exactly once', async () => {
    const { adapter, created, revoked } = harness(new AnimationClient(3))
    const { session, imported } = await importDemo(adapter)
    const artifact = await adapter.preview(session, imported.source, DEFAULT_SETTINGS, context(2))
    expect(revoked).toEqual([])
    artifact.dispose()
    artifact.dispose()
    expect(revoked).toEqual(created)
  })

  it('passes the export frame to the client and rejects an out-of-range frame', async () => {
    const client = new AnimationClient(3)
    const { adapter } = harness(client)
    const { session, imported } = await importDemo(adapter)
    const artifact = await adapter.export(
      session,
      imported.source,
      'text',
      DEFAULT_SETTINGS,
      context(3),
      undefined,
      { frame: 2 },
    )
    expect(artifact.kind).toBe('blob')
    expect(client.exportFrames).toEqual([2])

    await expect(
      adapter.export(session, imported.source, 'text', DEFAULT_SETTINGS, context(4), undefined, {
        frame: 3,
      }),
    ).rejects.toThrow(RangeError)
    expect(client.exportFrames).toEqual([2])
  })

  it('exports frame 0 when no frame is given', async () => {
    const client = new AnimationClient(2)
    const { adapter } = harness(client)
    const { session, imported } = await importDemo(adapter)
    await adapter.export(session, imported.source, 'text', DEFAULT_SETTINGS, context(3))
    expect(client.exportFrames).toEqual([0])
  })
})
