import { describe, expect, it } from 'vitest'
import type {
  ExportFormat,
  ExportRenderOptions,
  ImportedSource,
  ImportedResult,
  PreviewKind,
  PreviewRenderOptions,
  ProcessingAdapter,
  ProcessingSession,
  StudioAnimation,
} from '../processing/types'
import { createStudioController } from './controller'

interface FakeAdapter {
  readonly adapter: ProcessingAdapter
  readonly previews: Array<{ readonly kind: PreviewKind; readonly options?: PreviewRenderOptions }>
  readonly exports: Array<{ readonly format: ExportFormat; readonly options?: ExportRenderOptions }>
}

/** Serves `frameCount` frames per import; a count of 1 imports a still. */
function animatedAdapter(frameCount: number): FakeAdapter {
  const previews: FakeAdapter['previews'] = []
  const exports: FakeAdapter['exports'] = []
  const delays = frameCount > 1 ? [5, 10, 2].slice(0, frameCount) : []
  const animation: StudioAnimation | null =
    frameCount > 1 ? { frameCount, delaysCs: delays, loopCount: 0 } : null
  const adapter: ProcessingAdapter = {
    createSession: () => ({}) as ProcessingSession,
    import: (): Promise<ImportedResult> =>
      Promise.resolve({
        source: {} as ImportedSource,
        metadata: { name: 'anim.gif', width: 2, height: 1, source: 'file', inputBytes: 3 },
        animation,
        dispose: () => undefined,
      }),
    preview: (_session, _source, _settings, _context, kind = 'artwork', _progress, options) => {
      previews.push({ kind, ...(options === undefined ? {} : { options }) })
      const urls = Array.from({ length: frameCount }, (_, frame) => `blob:${kind}-${frame}`)
      const start = options?.startFrame ?? 0
      return Promise.resolve({
        url: urls[start] ?? '',
        width: 2,
        height: 1,
        ...(frameCount > 1 ? { frames: urls } : {}),
        dispose: () => undefined,
      })
    },
    export: (_session, _source, format, _settings, _context, _progress, options) => {
      exports.push({ format, ...(options === undefined ? {} : { options }) })
      return Promise.resolve({
        kind: 'blob' as const,
        format,
        blob: new Blob(['x']),
        byteLength: 1,
        dispose: () => undefined,
      })
    },
    cancel: () => undefined,
    dispose: () => undefined,
  }
  return { adapter, previews, exports }
}

describe('controller animation snapshot', () => {
  it('sets a frozen animation on import and clears it on reset and dispose', async () => {
    const { adapter } = animatedAdapter(3)
    const controller = createStudioController({ adapter })
    expect(controller.getSnapshot().animation).toBeNull()

    await controller.import({ kind: 'demo' })
    const animation = controller.getSnapshot().animation
    expect(animation).toEqual({ frameCount: 3, delaysCs: [5, 10, 2], loopCount: 0 })
    expect(Object.isFrozen(animation)).toBe(true)
    expect(Object.isFrozen(animation?.delaysCs)).toBe(true)

    controller.reset()
    expect(controller.getSnapshot().animation).toBeNull()

    await controller.import({ kind: 'demo' })
    expect(controller.getSnapshot().animation).not.toBeNull()
    controller.dispose()
    expect(controller.getSnapshot().animation).toBeNull()
  })

  it('leaves the animation null for a still import', async () => {
    const controller = createStudioController({ adapter: animatedAdapter(1).adapter })
    await controller.import({ kind: 'demo' })
    expect(controller.getSnapshot().animation).toBeNull()
  })

  it('publishes frame urls for an animated preview and passes the start frame through', async () => {
    const { adapter, previews } = animatedAdapter(3)
    const controller = createStudioController({ adapter })
    await controller.import({ kind: 'demo' })

    await controller.preview('artwork', { startFrame: 2 })
    expect(previews.at(-1)).toEqual({ kind: 'artwork', options: { startFrame: 2 } })
    expect(controller.getSnapshot().preview).toEqual({
      url: 'blob:artwork-2',
      width: 2,
      height: 1,
      frames: ['blob:artwork-0', 'blob:artwork-1', 'blob:artwork-2'],
    })
    expect(Object.isFrozen(controller.getSnapshot().preview?.frames)).toBe(true)

    await controller.preview('original')
    expect(previews.at(-1)).toEqual({ kind: 'original', options: { startFrame: 0 } })
    expect(controller.getSnapshot().original?.frames).toHaveLength(3)
  })

  it('publishes no frames for a still preview', async () => {
    const controller = createStudioController({ adapter: animatedAdapter(1).adapter })
    await controller.import({ kind: 'demo' })
    await controller.preview()
    expect(controller.getSnapshot().preview).not.toHaveProperty('frames')
    expect(controller.getSnapshot().preview?.url).toBe('blob:artwork-0')
  })

  it('rejects an out-of-range start frame before any state changes', async () => {
    const { adapter, previews } = animatedAdapter(3)
    const controller = createStudioController({ adapter })
    await controller.import({ kind: 'demo' })
    const before = controller.getSnapshot()

    for (const startFrame of [3, -1, 1.5, Number.NaN]) {
      await expect(controller.preview('artwork', { startFrame })).rejects.toThrow(RangeError)
    }
    expect(controller.getSnapshot()).toBe(before)
    expect(previews).toEqual([])
  })

  it('rejects any non-zero start frame for a still', async () => {
    const controller = createStudioController({ adapter: animatedAdapter(1).adapter })
    await controller.import({ kind: 'demo' })
    const before = controller.getSnapshot()
    await expect(controller.preview('artwork', { startFrame: 1 })).rejects.toThrow(RangeError)
    expect(controller.getSnapshot()).toBe(before)
  })

  it('passes the export frame and rejects an out-of-range one before export state changes', async () => {
    const { adapter, exports } = animatedAdapter(3)
    const controller = createStudioController({ adapter })
    await controller.import({ kind: 'demo' })

    await controller.startExport('png', { frame: 2 })
    expect(exports.at(-1)).toEqual({ format: 'png', options: { frame: 2 } })
    expect(controller.getSnapshot().export.status).toBe('ready')

    const before = controller.getSnapshot()
    await expect(controller.startExport('png', { frame: 3 })).rejects.toThrow(RangeError)
    expect(controller.getSnapshot()).toBe(before)
    expect(exports).toHaveLength(1)
  })

  it('exports frame 0 when no frame is given', async () => {
    const { adapter, exports } = animatedAdapter(2)
    const controller = createStudioController({ adapter })
    await controller.import({ kind: 'demo' })
    await controller.startExport('text')
    expect(exports.at(-1)).toEqual({ format: 'text', options: { frame: 0 } })
  })
})
