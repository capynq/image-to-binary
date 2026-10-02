export type StudioJobStatus =
  | 'idle'
  | 'importing'
  | 'ready'
  | 'processing'
  | 'exporting'
  | 'cancelled'
  | 'error'
  | 'disposed'

export type ExportFormat = 'png' | 'text'

/** Immutable settings shared by preview and export operations. */
export interface StudioSettings {
  readonly cellSize: number
  readonly fontSize: number
  readonly threshold: number
}

export interface StudioMetadata {
  readonly name: string
  readonly width: number
  readonly height: number
  readonly source: 'demo'
}

export interface DemoFixtureInput {
  readonly kind: 'demo'
}

export type ImportInput = DemoFixtureInput | File

export interface ImportedResult {
  readonly source: ImportedSource
  readonly metadata: StudioMetadata
}

export interface PreviewSnapshot {
  readonly url: string
  readonly width: number
  readonly height: number
}

/** JSON-safe view state; binary payloads and revocable handles stay private. */
export interface StudioSnapshot {
  readonly status: StudioJobStatus
  readonly generation: number
  readonly progress: number
  readonly settings: StudioSettings
  readonly metadata: StudioMetadata | null
  readonly preview: PreviewSnapshot | null
  readonly error: string | null
}

export interface JobContext {
  readonly operationId: number
  readonly generation: number
}

export interface ProcessingSession {
  readonly __processingSession: unique symbol
}

export interface ImportedSource {
  readonly __importedSource: unique symbol
}

export interface PreviewArtifact {
  readonly url: string
  readonly width: number
  readonly height: number
  dispose(): void
}

/** Adapter boundary for import, deterministic rendering, export, and lifecycle ownership. */
export interface ProcessingAdapter {
  createSession(): ProcessingSession
  import(
    session: ProcessingSession,
    input: ImportInput,
    context: JobContext,
  ): Promise<ImportedResult>
  preview(
    session: ProcessingSession,
    source: ImportedSource,
    settings: StudioSettings,
    context: JobContext,
  ): Promise<PreviewArtifact>
  export(
    session: ProcessingSession,
    source: ImportedSource,
    format: ExportFormat,
    settings: StudioSettings,
    context: JobContext,
  ): Promise<Blob>
  cancel(session: ProcessingSession, operationId: number): void
  dispose(session: ProcessingSession): void
}

export interface UrlApi {
  createObjectURL(object: Blob): string
  revokeObjectURL(url: string): void
}

export const DEFAULT_SETTINGS: StudioSettings = Object.freeze({
  cellSize: 12,
  fontSize: 16,
  threshold: 128,
})

export function cloneSettings(settings: StudioSettings): StudioSettings {
  return Object.freeze({ ...settings })
}

export function validateSettings(settings: StudioSettings): void {
  if (!Number.isInteger(settings.cellSize) || settings.cellSize < 5 || settings.cellSize > 30) {
    throw new RangeError('Cell size must be an integer from 5 through 30.')
  }
  if (!Number.isInteger(settings.fontSize) || settings.fontSize < 8 || settings.fontSize > 30) {
    throw new RangeError('Font size must be an integer from 8 through 30.')
  }
  if (!Number.isInteger(settings.threshold) || settings.threshold < 0 || settings.threshold > 255) {
    throw new RangeError('Threshold must be an integer from 0 through 255.')
  }
}
