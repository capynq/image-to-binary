import { createDemoImage } from './demoImage'
import { defaultUrlApi, isDemoFixture, UnsupportedInputError } from './demoAdapter'
import { renderPreviewPng } from './previewRenderer'
import type { PreviewRenderer } from './previewRenderer'
import type {
  ImportedResult,
  ImportedSource,
  PreviewArtifact,
  ProcessingAdapter,
  ProcessingSession,
  UrlApi,
} from './types'
import { validateSettings } from './types'
import type { PreviewPixels, ProcessingClient, RgbaImport, SourceHandle } from './workerClient'

export class ExportUnavailableError extends Error {
  constructor() {
    super('Export is not available until the PNG and text export phases.')
    this.name = 'ExportUnavailableError'
  }
}

export interface BrowserAdapterOptions {
  /** Invoked lazily on the first import; session creation allocates nothing heavy. */
  readonly createClient: () => ProcessingClient
  readonly renderPreview?: PreviewRenderer
  readonly urlApi?: UrlApi
  readonly createDemoImage?: () => RgbaImport
}

interface SourceRecord {
  readonly handle: SourceHandle
  released: boolean
}

interface SessionState {
  disposed: boolean
  client: ProcessingClient | null
  readonly sources: WeakMap<ImportedSource, SourceRecord>
}

/**
 * Adapter backed by the Rust WASM worker client. Preview pixels are encoded to
 * a PNG object URL on the main thread; export is explicitly unavailable.
 */
export function createBrowserProcessingAdapter(options: BrowserAdapterOptions): ProcessingAdapter {
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
      })
      return session
    },
    async import(session, input, context): Promise<ImportedResult> {
      const state = assertSession(session)
      assertActive(state)
      if (!isDemoFixture(input)) {
        throw new UnsupportedInputError()
      }
      const handle = await clientOf(state).importRgba(makeDemoImage, context.operationId)
      if (state.disposed) {
        handle.dispose()
        throw new Error('Processing session is disposed.')
      }
      const source = {} as ImportedSource
      const record: SourceRecord = { handle, released: false }
      state.sources.set(source, record)
      return {
        source,
        metadata: { name: handle.name, width: handle.width, height: handle.height, source: 'demo' },
        dispose(): void {
          if (!record.released) {
            record.released = true
            state.sources.delete(source)
            handle.dispose()
          }
        },
      }
    },
    async preview(session, source, settings, context): Promise<PreviewArtifact> {
      const state = assertSession(session)
      assertActive(state)
      const record = sourceOf(state, source)
      validateSettings(settings)
      return clientOf(state).preview(
        record.handle,
        {
          cellSize: settings.cellSize,
          fontSize: settings.fontSize,
          threshold: settings.threshold,
          sampling: 'classic',
        },
        context.operationId,
        createArtifact,
      )
    },
    async export(session): Promise<Blob> {
      assertActive(assertSession(session))
      throw new ExportUnavailableError()
    },
    cancel(session, operationId): void {
      assertSession(session).client?.cancel(operationId)
    },
    dispose(session): void {
      const state = assertSession(session)
      if (state.disposed) return
      state.disposed = true
      const client = state.client
      state.client = null
      client?.dispose()
    },
  }
}
