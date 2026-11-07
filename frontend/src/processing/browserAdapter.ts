import { createDemoImage } from './demoImage'
import { defaultUrlApi, isDemoFixture, UnsupportedInputError } from './demoAdapter'
import { createExportAccumulator } from './exportAccumulator'
import type { ExportAccumulator, ExportAccumulatorOptions } from './exportAccumulator'
import { startHeifDecode } from './heifDecodeJob'
import type { DecodedRgba, HeifDecodeJob, HeifDecoderEndpoint } from './heifDecodeJob'
import { isOutputKindName, validateEncodedImport } from './protocol'
import { renderPreviewPng } from './previewRenderer'
import type { PreviewRenderer } from './previewRenderer'
import type {
  ExportArtifact,
  ExportProgressListener,
  ImportInput,
  ImportedResult,
  ImportedSource,
  ImportStageListener,
  PreviewArtifact,
  PreviewKind,
  ProcessingAdapter,
  ProcessingSession,
  StudioMetadata,
  UrlApi,
} from './types'
import { previewParametersOf, validateSettings } from './types'
import { ProcessingWorkerError } from './workerClient'
import type { PreviewPixels, ProcessingClient, RgbaImport, SourceHandle } from './workerClient'

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

interface SessionState {
  disposed: boolean
  client: ProcessingClient | null
  readonly sources: WeakMap<ImportedSource, SourceRecord>
  readonly decodes: Map<number, HeifDecodeJob>
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

  return {
    createSession(): ProcessingSession {
      const session = {} as ProcessingSession
      sessions.set(session, {
        disposed: false,
        client: null,
        sources: new WeakMap<ImportedSource, SourceRecord>(),
        decodes: new Map<number, HeifDecodeJob>(),
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
    ): Promise<PreviewArtifact> {
      const state = assertSession(session)
      assertActive(state)
      const record = sourceOf(state, source)
      if (kind === 'original') {
        return clientOf(state).originalPreview(record.handle, context.operationId, createArtifact)
      }
      if (kind !== 'artwork') {
        throw new RangeError('Unknown preview kind.')
      }
      validateSettings(settings)
      return clientOf(state).preview(
        record.handle,
        previewParametersOf(settings),
        context.operationId,
        createArtifact,
      )
    },
    async export(
      session,
      source,
      format,
      settings,
      context,
      onProgress?: ExportProgressListener,
    ): Promise<ExportArtifact> {
      const state = assertSession(session)
      assertActive(state)
      const record = sourceOf(state, source)
      if (!isOutputKindName(format)) {
        throw new RangeError('Export format must be png or text.')
      }
      validateSettings(settings)
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
        )
      } finally {
        accumulator.release()
      }
    },
    cancel(session, operationId): void {
      const state = assertSession(session)
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

function isCodecUnavailable(error: unknown): boolean {
  return error instanceof ProcessingWorkerError && error.code === 'codec_unavailable'
}

/** Narrows an import input to a real `File`; MIME type and extension are never consulted. */
function isFileInput(input: ImportInput): input is File {
  return typeof File === 'function' && input instanceof File
}
