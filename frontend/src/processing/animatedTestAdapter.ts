import type {
  ExportFormat,
  ExportRenderOptions,
  ImportedResult,
  ImportedSource,
  PreviewKind,
  PreviewRenderOptions,
  ProcessingAdapter,
  ProcessingSession,
  StudioAnimation,
} from './types'

export interface AnimatedTestAdapter {
  readonly adapter: ProcessingAdapter
  readonly previews: Array<{ readonly kind: PreviewKind; readonly options?: PreviewRenderOptions }>
  readonly exports: Array<{ readonly format: ExportFormat; readonly options?: ExportRenderOptions }>
}

export interface AnimatedTestOptions {
  /** Frame count; 1 imports a still. */
  readonly frameCount: number
  readonly delaysCs?: ReadonlyArray<number>
  readonly loopCount?: number
}

/**
 * Processing adapter that imports an animation of `frameCount` frames without decoding anything.
 * Preview frame URLs are `blob:<kind>-<index>`, so tests can read which frame each view shows.
 */
export function animatedTestAdapter({
  frameCount,
  delaysCs,
  loopCount = 0,
}: AnimatedTestOptions): AnimatedTestAdapter {
  const previews: AnimatedTestAdapter['previews'] = []
  const exports: AnimatedTestAdapter['exports'] = []
  const delays = delaysCs ?? Array.from({ length: frameCount }, () => 5)
  const animation: StudioAnimation | null =
    frameCount > 1 ? { frameCount, delaysCs: delays, loopCount } : null
  const adapter: ProcessingAdapter = {
    createSession: () => ({}) as ProcessingSession,
    import: (): Promise<ImportedResult> =>
      Promise.resolve({
        source: {} as ImportedSource,
        metadata: { name: 'animated.gif', width: 2, height: 1, source: 'file', inputBytes: 3 },
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
