import type {
  DemoFixtureInput,
  ExportArtifact,
  ImportedSource,
  ImportedResult,
  ImportInput,
  JobContext,
  PreviewArtifact,
  PreviewKind,
  ProcessingAdapter,
  ProcessingSession,
  StudioSettings,
  UrlApi,
} from './types'
import { validateSettings } from './types'

const DEMO_CHARS = ['1010', '0101', '1010', '0101'] as const
const DEMO_LUMA = [
  [32, 224, 32, 224],
  [224, 32, 224, 32],
  [32, 224, 32, 224],
  [224, 32, 224, 32],
] as const

interface OperationGuard {
  readonly operationId: number
  cancelled: boolean
}

interface DemoSessionState {
  disposed: boolean
  readonly sources: WeakMap<ImportedSource, true>
  activeOperation: OperationGuard | null
}

interface DemoSource extends ImportedSource {
  readonly __demoSource: true
}

export interface DemoAdapterOptions {
  readonly urlApi?: UrlApi
  readonly delay?: (milliseconds: number) => Promise<void>
}

export class UnsupportedInputError extends Error {
  constructor() {
    super('File conversion is not available until the file import phase.')
    this.name = 'UnsupportedInputError'
  }
}

export class UnsupportedExportError extends Error {
  constructor() {
    super('PNG export is not available until the PNG export phase.')
    this.name = 'UnsupportedExportError'
  }
}

export class UnsupportedPreviewKindError extends Error {
  constructor() {
    super('Original preview is not available in the demo-only adapter.')
    this.name = 'UnsupportedPreviewKindError'
  }
}

export function createDemoProcessingAdapter(options: DemoAdapterOptions = {}): ProcessingAdapter {
  const urlApi = options.urlApi ?? defaultUrlApi()
  const delay = options.delay ?? (async () => undefined)
  const sessions = new WeakMap<ProcessingSession, DemoSessionState>()
  const assertSession = (session: ProcessingSession): DemoSessionState => {
    const state = sessions.get(session)
    if (state === undefined) {
      throw new Error('Processing session does not belong to this adapter.')
    }
    return state
  }

  return {
    createSession(): ProcessingSession {
      const session = {} as ProcessingSession
      sessions.set(session, {
        disposed: false,
        sources: new WeakMap<ImportedSource, true>(),
        activeOperation: null,
      })
      return session
    },
    async import(session, input, context): Promise<ImportedResult> {
      const demoSession = assertSession(session)
      assertActive(demoSession)
      const guard = beginOperation(demoSession, context)
      try {
        if (!isDemoFixture(input)) {
          throw new UnsupportedInputError()
        }
        assertOperationActive(demoSession, context, guard)
        const source = { __importedSource: Symbol('demo-source'), __demoSource: true } as DemoSource
        demoSession.sources.set(source, true)
        let released = false
        return {
          source,
          metadata: { name: 'Demo fixture', width: 4, height: 4, source: 'demo' },
          dispose(): void {
            if (!released) {
              released = true
              demoSession.sources.delete(source)
            }
          },
        }
      } finally {
        finishOperation(demoSession, guard)
      }
    },
    async preview(
      session,
      source,
      settings,
      context,
      kind: PreviewKind = 'artwork',
    ): Promise<PreviewArtifact> {
      const demoSession = assertSession(session)
      assertActive(demoSession)
      assertSource(demoSession, source)
      if (kind === 'original') {
        throw new UnsupportedPreviewKindError()
      }
      if (kind !== 'artwork') {
        throw new RangeError('Unknown preview kind.')
      }
      validateSettings(settings)
      const guard = beginOperation(demoSession, context)
      try {
        await delay(0)
        assertOperationActive(demoSession, context, guard)
        const chars = fixtureChars(settings.threshold)
        const svg = createDemoSvg(settings, chars)
        const url = urlApi.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }))
        let disposed = false
        return {
          url,
          width: DEMO_CHARS[0].length * settings.cellSize,
          height: DEMO_CHARS.length * settings.cellSize,
          dispose(): void {
            if (!disposed) {
              disposed = true
              urlApi.revokeObjectURL(url)
            }
          },
        }
      } finally {
        finishOperation(demoSession, guard)
      }
    },
    async export(session, source, format, settings, context): Promise<ExportArtifact> {
      const demoSession = assertSession(session)
      assertActive(demoSession)
      assertSource(demoSession, source)
      validateSettings(settings)
      if (format === 'png') {
        throw new UnsupportedExportError()
      }
      const guard = beginOperation(demoSession, context)
      try {
        await delay(0)
        assertOperationActive(demoSession, context, guard)
        const blob = new Blob([fixtureChars(settings.threshold).join('\n')], {
          type: 'text/plain;charset=utf-8',
        })
        return { kind: 'blob', blob, byteLength: blob.size, dispose: () => undefined }
      } finally {
        finishOperation(demoSession, guard)
      }
    },
    cancel(session, operationId): void {
      const demoSession = assertSession(session)
      if (!demoSession.disposed) {
        if (demoSession.activeOperation?.operationId === operationId) {
          demoSession.activeOperation.cancelled = true
        }
      }
    },
    dispose(session): void {
      const demoSession = assertSession(session)
      demoSession.disposed = true
      if (demoSession.activeOperation !== null) {
        demoSession.activeOperation.cancelled = true
        demoSession.activeOperation = null
      }
    },
  }
}

export function isDemoFixture(input: ImportInput): input is DemoFixtureInput {
  return typeof input === 'object' && input !== null && 'kind' in input && input.kind === 'demo'
}

export function defaultUrlApi(): UrlApi {
  return {
    createObjectURL: (object) => URL.createObjectURL(object),
    revokeObjectURL: (url) => URL.revokeObjectURL(url),
  }
}

function assertActive(session: DemoSessionState): void {
  if (session.disposed) {
    throw new Error('Processing session is disposed.')
  }
}

function assertSource(session: DemoSessionState, source: ImportedSource): void {
  if (!session.sources.has(source)) {
    throw new Error('Imported source does not belong to this processing session.')
  }
}

function beginOperation(session: DemoSessionState, context: JobContext): OperationGuard {
  if (session.activeOperation !== null) {
    session.activeOperation.cancelled = true
  }
  const guard = { operationId: context.operationId, cancelled: false }
  session.activeOperation = guard
  return guard
}

function finishOperation(session: DemoSessionState, guard: OperationGuard): void {
  if (session.activeOperation === guard) {
    session.activeOperation = null
  }
}

function assertOperationActive(
  session: DemoSessionState,
  context: JobContext,
  guard: OperationGuard,
): void {
  assertActive(session)
  if (
    guard.cancelled ||
    session.activeOperation !== guard ||
    guard.operationId !== context.operationId
  ) {
    throw new Error('Processing operation was cancelled.')
  }
}

/** Browser-only fixture raster; Rust fontdue parity is deferred to the worker phase. */
function createDemoSvg(settings: StudioSettings, chars: readonly string[]): string {
  const width = DEMO_CHARS[0].length * settings.cellSize
  const height = DEMO_CHARS.length * settings.cellSize
  const cells = chars
    .flatMap((row, y) =>
      [...row].map((character, x) => {
        const baseline = y * settings.cellSize + Math.round(settings.cellSize * 0.8)
        return `<text x="${x * settings.cellSize + Math.round(settings.cellSize * 0.16)}" y="${baseline}" fill="#00ff00" font-family="monospace" font-size="${settings.fontSize}">${character}</text>`
      }),
    )
    .join('')
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="${width}" height="${height}" fill="#000000"/>${cells}</svg>`
}

function fixtureChars(threshold: number): string[] {
  return DEMO_LUMA.map((row) => row.map((luma) => (luma <= threshold ? '1' : '0')).join(''))
}
