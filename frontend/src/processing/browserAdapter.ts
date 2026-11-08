import { createDemoImage } from './demoImage'
import { defaultUrlApi, isDemoFixture, UnsupportedInputError } from './demoAdapter'
import { createExportAccumulator } from './exportAccumulator'
import type { ExportAccumulator, ExportAccumulatorOptions } from './exportAccumulator'
import { startHeifDecode } from './heifDecodeJob'
import type { DecodedRgba, HeifDecodeJob, HeifDecoderEndpoint } from './heifDecodeJob'
import { isOutputKindName, validateEncodedImport, validateFrameIndex } from './protocol'
import { renderPreviewPng } from './previewRenderer'
import type { PreviewRenderer } from './previewRenderer'
import type {
  ExportArtifact,
  ExportProgressListener,
  ExportRenderOptions,
  ImportInput,
  ImportedResult,
  ImportedSource,
  ImportStageListener,
  PreviewArtifact,
  PreviewKind,
  PreviewProgressListener,
  PreviewRenderOptions,
  ProcessingAdapter,
  ProcessingSession,
  StudioMetadata,
  UrlApi,
} from './types'
import { freezeAnimation, previewParametersOf, validateSettings } from './types'
import { ProcessingWorkerError } from './workerClient'
import type { PreviewPixels, ProcessingClient, RgbaImport, SourceHandle } from './workerClient'

/** Total encoded PNG bytes one animation preview may retain across all of its frames. */
export const ANIMATION_PREVIEW_BUDGET_BYTES = 512 * 1024 * 1024
export const ANIMATION_BUDGET_MESSAGE = 'Animation preview is too large; try a larger cell size.'

export interface BrowserAdapterOptions {
  /** Invoked lazily on the first import; session creation allocates nothing heavy. */
  readonly createClient: () => ProcessingClient
  readonly renderPreview?: PreviewRenderer
  readonly urlApi?: UrlApi
  readonly createDemoImage?: () => RgbaImport
  /**
   * Creates one dedicated decoder worker per HEIC/AVIF fallback, which runs only after the
   * processing worker reports `codec_unavailable`. Omitted, that error propagates unchanged.
   */
  readonly createHeifDecoderEndpoint?: () => HeifDecoderEndpoint
}

interface SourceRecord {
  readonly handle: SourceHandle
  released: boolean
}

/** One multi-frame preview in flight; `cancelled` is read between frames. */
interface AnimationRun {
  cancelled: boolean
  retainedBytes: number
}

/** A PNG frame owned by its animation run; `dispose` is idempotent. */
interface EncodedFrame extends PreviewArtifact {
  readonly byteLength: number
}

interface SessionState {
  disposed: boolean
  client: ProcessingClient | null
  readonly sources: WeakMap<ImportedSource, SourceRecord>
  readonly decodes: Map<number, HeifDecodeJob>
  readonly runs: Map<number, AnimationRun>
}

/**
 * Adapter backed by the Rust WASM worker client. Preview pixels are encoded to
 * a PNG object URL on the main thread. PNG and UTF-8 text exports stream through the
 * shared scheduler into an adapter-owned accumulator, which builds the Blob of the format.
 */
export function createBrowserProcessingAdapter(options: BrowserAdapterOptions): ProcessingAdapter {
  return createAdapterWithAccumulator(options, createExportAccumulator)
}

/**
 * Internal seam, not re-exported from the package index: `makeAccumulator` builds the owner of
 * each export's chunks, so tests can observe release. Production code uses the public factory.
 */
export function createAdapterWithAccumulator(
  options: BrowserAdapterOptions,
  makeAccumulator: (accumulatorOptions: ExportAccumulatorOptions) => ExportAccumulator,
): ProcessingAdapter {
  const urlApi = options.urlApi ?? defaultUrlApi()
  const renderPreview = options.renderPreview ?? renderPreviewPng
  const makeDemoImage = options.createDemoImage ?? createDemoImage
  const sessions = new WeakMap<ProcessingSession, SessionState>()

  const assertSession = (session: ProcessingSession): SessionState => {
    const state = sessions.get(session)
    if (state === undefined) {
      throw new Error('Processing session does not belong to this adapter.')
    }
    return state
  }
  const assertActive = (state: SessionState): void => {
    if (state.disposed) throw new Error('Processing session is disposed.')
  }
  const clientOf = (state: SessionState): ProcessingClient => {
    assertActive(state)
    state.client ??= options.createClient()
    return state.client
  }
  const sourceOf = (state: SessionState, source: ImportedSource): SourceRecord => {
    const record = state.sources.get(source)
    if (record === undefined || record.released) {
      throw new Error('Imported source does not belong to this processing session.')
    }
    return record
  }
  const retain = (
    state: SessionState,
    handle: SourceHandle,
    metadata: StudioMetadata,
  ): ImportedResult => {
    if (state.disposed) {
      handle.dispose()
      throw new Error('Processing session is disposed.')
    }
    const source = {} as ImportedSource
    const record: SourceRecord = { handle, released: false }
    state.sources.set(source, record)
    return {
      source,
      metadata,
      animation: freezeAnimation({
        frameCount: handle.frameCount,
        delaysCs: handle.frameDelays,
        loopCount: handle.loopCount,
      }),
      dispose(): void {
        if (!record.released) {
          record.released = true
          state.sources.delete(source)
          handle.dispose()
        }
      },
    }
  }
  const cancelDecodes = (state: SessionState): void => {
    const jobs = [...state.decodes.values()]
    state.decodes.clear()
    for (const job of jobs) job.cancel()
  }
  const importWithHeifDecoder = async (
    state: SessionState,
    file: File,
    operationId: number,
    createEndpoint: () => HeifDecoderEndpoint,
    onStage: ImportStageListener | undefined,
  ): Promise<ImportedResult> => {
    onStage?.('heifDecoder')
    const job = startHeifDecode(file, createEndpoint)
    state.decodes.set(operationId, job)
    let decoded: DecodedRgba
    try {
      decoded = await job.result
    } catch (error) {
      if (state.decodes.get(operationId) === job) state.decodes.delete(operationId)
      throw error
    }
    if (state.decodes.get(operationId) !== job) {
      throw new ProcessingWorkerError('cancelled', 'HEIC and AVIF decoding was cancelled.')
    }
    state.decodes.delete(operationId)
    const handle = await clientOf(state).importRgba(
      {
        name: file.name,
        width: decoded.width,
        height: decoded.height,
        pixels: decoded.pixels,
      },
      operationId,
    )
    return retain(state, handle, {
      name: handle.name,
      width: handle.width,
      height: handle.height,
      source: 'file',
      inputBytes: file.size,
    })
  }
  const createArtifact = async (pixels: PreviewPixels): Promise<PreviewArtifact> => {
    const blob = await renderPreview(pixels)
    const url = urlApi.createObjectURL(blob)
    let revoked = false
    return {
      url,
      width: pixels.width,
      height: pixels.height,
      dispose(): void {
        if (!revoked) {
          revoked = true
          urlApi.revokeObjectURL(url)
        }
      },
    }
  }
  /**
   * Encodes one animation frame. The budget is checked on the encoded size before its object URL
   * exists, so a rejected frame never leaves a URL behind.
   */
  const encodeFrame =
    (run: AnimationRun) =>
    async (pixels: PreviewPixels): Promise<EncodedFrame> => {
      const blob = await renderPreview(pixels)
      if (run.retainedBytes + blob.size > ANIMATION_PREVIEW_BUDGET_BYTES) {
        throw new RangeError(ANIMATION_BUDGET_MESSAGE)
      }
      const url = urlApi.createObjectURL(blob)
      let revoked = false
      return {
        url,
        width: pixels.width,
        height: pixels.height,
        byteLength: blob.size,
        dispose(): void {
          if (!revoked) {
            revoked = true
            urlApi.revokeObjectURL(url)
          }
        },
      }
    }
  /**
   * Renders every frame in wrap-around order from `startFrame`. A cancel or a failure revokes each
   * frame already made; a finished run returns the start frame's fields plus all URLs in frame order.
   */
  const renderAnimation = async (
    state: SessionState,
    operationId: number,
    frameCount: number,
    startFrame: number,
    onProgress: PreviewProgressListener | undefined,
    renderFrame: (frame: number, run: AnimationRun) => Promise<EncodedFrame>,
  ): Promise<PreviewArtifact> => {
    const run: AnimationRun = { cancelled: false, retainedBytes: 0 }
    state.runs.set(operationId, run)
    const encoded: EncodedFrame[] = []
    try {
      for (let step = 0; step < frameCount; step += 1) {
        assertNotCancelled(run)
        const frame = (startFrame + step) % frameCount
        const artifact = await renderFrame(frame, run)
        encoded[frame] = artifact
        run.retainedBytes += artifact.byteLength
        onProgress?.((step + 1) / frameCount)
      }
      assertNotCancelled(run)
    } catch (error) {
      encoded.forEach((artifact) => artifact.dispose())
      throw run.cancelled ? cancelledError() : error
    } finally {
      if (state.runs.get(operationId) === run) state.runs.delete(operationId)
    }
    const start = encoded[startFrame]
    if (start === undefined) throw new Error('The animation preview lost its start frame.')
    return {
      url: start.url,
      width: start.width,
      height: start.height,
      frames: Object.freeze(encoded.map((artifact) => artifact.url)),
      dispose(): void {
        encoded.forEach((artifact) => artifact.dispose())
      },
    }
  }

  return {
    createSession(): ProcessingSession {
      const session = {} as ProcessingSession
      sessions.set(session, {
        disposed: false,
        client: null,
        sources: new WeakMap<ImportedSource, SourceRecord>(),
        decodes: new Map<number, HeifDecodeJob>(),
        runs: new Map<number, AnimationRun>(),
      })
      return session
    },
    async import(session, input, context, onStage?: ImportStageListener): Promise<ImportedResult> {
      const state = assertSession(session)
      assertActive(state)
      if (isDemoFixture(input)) {
        const handle = await clientOf(state).importRgba(makeDemoImage, context.operationId)
        return retain(state, handle, {
          name: handle.name,
          width: handle.width,
          height: handle.height,
          source: 'demo',
        })
      }
      if (!isFileInput(input)) {
        throw new UnsupportedInputError()
      }
      const file = input
      const name = file.name
      const inputBytes = file.size
      const invalid = validateEncodedImport(name, inputBytes)
      if (invalid !== null) throw new RangeError(invalid.message)
      let handle: SourceHandle
      try {
        handle = await clientOf(state).importEncoded(
          async () => ({ name, bytes: await file.arrayBuffer() }),
          context.operationId,
        )
      } catch (error) {
        const createEndpoint = options.createHeifDecoderEndpoint
        if (!isCodecUnavailable(error) || createEndpoint === undefined) throw error
        return importWithHeifDecoder(state, file, context.operationId, createEndpoint, onStage)
      }
      return retain(state, handle, {
        name: handle.name,
        width: handle.width,
        height: handle.height,
        source: 'file',
        inputBytes,
      })
    },
    async preview(
      session,
      source,
      settings,
      context,
      kind: PreviewKind = 'artwork',
      onProgress?: PreviewProgressListener,
      options: PreviewRenderOptions = {},
    ): Promise<PreviewArtifact> {
      const state = assertSession(session)
      assertActive(state)
      const record = sourceOf(state, source)
      if (kind !== 'original' && kind !== 'artwork') {
        throw new RangeError('Unknown preview kind.')
      }
      const startFrame = options.startFrame ?? 0
      const frameCount = record.handle.frameCount
      const frameProblem = validateFrameIndex(startFrame, frameCount)
      if (frameProblem !== null) throw new RangeError(frameProblem)
      const client = clientOf(state)
      const operationId = context.operationId
      if (kind === 'original') {
        if (frameCount === 1) {
          return client.originalPreview(record.handle, operationId, createArtifact)
        }
        return renderAnimation(
          state,
          operationId,
          frameCount,
          startFrame,
          onProgress,
          (frame, run) =>
            client.originalPreview(record.handle, operationId, encodeFrame(run), frame),
        )
      }
      validateSettings(settings)
      const parameters = previewParametersOf(settings)
      if (frameCount === 1) {
        return client.preview(record.handle, parameters, operationId, createArtifact)
      }
      return renderAnimation(state, operationId, frameCount, startFrame, onProgress, (frame, run) =>
        client.preview(record.handle, parameters, operationId, encodeFrame(run), frame),
      )
    },
    async export(
      session,
      source,
      format,
      settings,
      context,
      onProgress?: ExportProgressListener,
      options: ExportRenderOptions = {},
    ): Promise<ExportArtifact> {
      const state = assertSession(session)
      assertActive(state)
      const record = sourceOf(state, source)
      if (!isOutputKindName(format)) {
        throw new RangeError('Export format must be png, text or gif.')
      }
      if (format === 'gif' && record.handle.frameCount === 1) {
        throw new RangeError('A GIF export needs an animated source.')
      }
      validateSettings(settings)
      const frame = options.frame ?? 0
      const frameProblem =
        format === 'gif' ? null : validateFrameIndex(frame, record.handle.frameCount)
      if (frameProblem !== null) throw new RangeError(frameProblem)
      const accumulator = makeAccumulator({
        format,
        ...(onProgress === undefined ? {} : { onProgress }),
      })
      try {
        return await clientOf(state).exportFile(
          record.handle,
          previewParametersOf(settings),
          format,
          context.operationId,
          accumulator,
          (completion) => accumulator.finish(completion),
          format === 'gif' ? undefined : frame,
        )
      } finally {
        accumulator.release()
      }
    },
    cancel(session, operationId): void {
      const state = assertSession(session)
      const run = state.runs.get(operationId)
      if (run !== undefined) run.cancelled = true
      const job = state.decodes.get(operationId)
      if (job !== undefined) {
        state.decodes.delete(operationId)
        job.cancel()
      }
      state.client?.cancel(operationId)
    },
    dispose(session): void {
      const state = assertSession(session)
      if (state.disposed) return
      state.disposed = true
      const client = state.client
      state.client = null
      cancelDecodes(state)
      client?.dispose()
    },
  }
}

function assertNotCancelled(run: AnimationRun): void {
  if (run.cancelled) throw cancelledError()
}

function cancelledError(): ProcessingWorkerError {
  return new ProcessingWorkerError('cancelled', 'Animation preview was cancelled.')
}

function isCodecUnavailable(error: unknown): boolean {
  return error instanceof ProcessingWorkerError && error.code === 'codec_unavailable'
}

/** Narrows an import input to a real `File`; MIME type and extension are never consulted. */
function isFileInput(input: ImportInput): input is File {
  return typeof File === 'function' && input instanceof File
}
