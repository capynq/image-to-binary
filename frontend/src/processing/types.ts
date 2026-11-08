import {
  COLOR_MODE_MESSAGE,
  DEFAULT_BACKGROUND,
  DEFAULT_FOREGROUND,
  DEFAULT_RAMP,
  isColorModeName,
  isRenderModeName,
  samplingForMode,
  validateColour,
  validateContrast,
  validateRamp,
  INVERT_BOOLEAN_MESSAGE,
} from './protocol'
import type { ColorModeName, PreviewParameters, RenderModeName } from './protocol'
import {
  MAX_CELL_SIZE,
  MAX_FONT_SIZE,
  MAX_THRESHOLD,
  MIN_CELL_SIZE,
  MIN_FONT_SIZE,
  MIN_THRESHOLD,
} from './bounds'

export type StudioJobStatus =
  | 'idle'
  | 'importing'
  | 'ready'
  | 'processing'
  | 'exporting'
  | 'cancelled'
  | 'error'
  | 'disposed'

/** `png` is the image file; `text` is UTF-8 grid text over the same grid and settings. */
export type ExportFormat = 'png' | 'text'

/** The Blob type of each export format; text is UTF-8 with no BOM. */
export const EXPORT_MIME_TYPES: Readonly<Record<ExportFormat, string>> = Object.freeze({
  png: 'image/png',
  text: 'text/plain;charset=utf-8',
})

export type ExportStatus = 'idle' | 'running' | 'ready' | 'failed'

/** JSON-safe export lifecycle; the encoded bytes live only in the adapter-owned artifact. */
export interface ExportSnapshot {
  readonly status: ExportStatus
  readonly progress: number
  readonly byteLength: number | null
  readonly error: string | null
}

/**
 * Immutable settings shared by preview and export operations. `ramp` is always a valid ramp:
 * the studio keeps an invalid draft out of this snapshot, so the last accepted ramp stays in use.
 */
export interface StudioSettings {
  readonly cellSize: number
  readonly fontSize: number
  readonly threshold: number
  readonly mode: RenderModeName
  readonly ramp: string
  readonly foreground: string
  readonly background: string
  readonly colorMode: ColorModeName
  readonly contrast: number
  readonly invert: boolean
}

export interface StudioMetadata {
  readonly name: string
  readonly width: number
  readonly height: number
  readonly source: 'demo' | 'file'
  /** Size in bytes of the encoded input file; absent for the demo fixture. */
  readonly inputBytes?: number
}

export interface DemoFixtureInput {
  readonly kind: 'demo'
}

/** Opaque handle to a file the native shell selected; the page never sees its path. */
export interface NativeFileInput {
  readonly kind: 'native-file'
  readonly id: number
  readonly name: string
  readonly byteLength: number
}

export type ImportInput = DemoFixtureInput | File | NativeFileInput

export interface ImportedResult {
  readonly source: ImportedSource
  readonly metadata: StudioMetadata
  /** Idempotently releases the source; the controller calls it once it stops owning the result. */
  dispose(): void
}

/** Sub-phase of a running import; `heifDecoder` while the lazy HEIC/AVIF worker decodes. */
export type ImportStage = 'heifDecoder'

export type ImportStageListener = (stage: ImportStage) => void

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
  readonly importStage: ImportStage | null
  readonly preview: PreviewSnapshot | null
  readonly original: PreviewSnapshot | null
  /** Increments on every accepted settings change; `previewRevision` says which one `preview` renders. */
  readonly settingsRevision: number
  /** The `settingsRevision` the displayed artwork was rendered from; null exactly when `preview` is null. */
  readonly previewRevision: number | null
  /** True while a quiet artwork render runs over the displayed artwork; status stays `ready`. */
  readonly artworkUpdating: boolean
  readonly export: ExportSnapshot
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

/**
 * Browser export: `blob` is the only handle to the encoded bytes, which never enter JSON state.
 * `format` names the file kind and fixes the Blob type.
 */
export interface BlobExportArtifact {
  readonly kind: 'blob'
  readonly format: ExportFormat
  readonly blob: Blob
  readonly byteLength: number
  dispose(): void
}

/** Native export already committed to a file the shell chose; no bytes cross into the page. */
export interface SavedExportArtifact {
  readonly kind: 'saved'
  readonly name: string
  readonly byteLength: number
  dispose(): void
}

export type ExportArtifact = BlobExportArtifact | SavedExportArtifact

/** Raised when the user dismisses the native save dialog; the export returns to idle silently. */
export class ExportCancelledError extends Error {
  constructor() {
    super('The export was cancelled before a destination was chosen.')
    this.name = 'ExportCancelledError'
  }
}

/** Receives a fraction in [0, 1]. */
export type ExportProgressListener = (fraction: number) => void

/** Receives a fraction in [0, 1] of a running preview; adapters that cannot report it omit it. */
export type PreviewProgressListener = (fraction: number) => void

/** `artwork` renders the converted output; `original` shows the bounded imported image. */
export type PreviewKind = 'artwork' | 'original'

/** Adapter boundary for import, deterministic rendering, export, and lifecycle ownership. */
export interface ProcessingAdapter {
  createSession(): ProcessingSession
  import(
    session: ProcessingSession,
    input: ImportInput,
    context: JobContext,
    onStage?: ImportStageListener,
  ): Promise<ImportedResult>
  preview(
    session: ProcessingSession,
    source: ImportedSource,
    settings: StudioSettings,
    context: JobContext,
    kind?: PreviewKind,
    onProgress?: PreviewProgressListener,
  ): Promise<PreviewArtifact>
  /**
   * Encodes `source` with `settings` as `format`. Rejects with a `RangeError` for any other
   * format; `onProgress` may run zero or more times before the result settles.
   */
  export(
    session: ProcessingSession,
    source: ImportedSource,
    format: ExportFormat,
    settings: StudioSettings,
    context: JobContext,
    onProgress?: ExportProgressListener,
  ): Promise<ExportArtifact>
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
  mode: 'binary',
  ramp: DEFAULT_RAMP,
  foreground: DEFAULT_FOREGROUND,
  background: DEFAULT_BACKGROUND,
  colorMode: 'monochrome',
  contrast: 0,
  invert: false,
})

export function cloneSettings(settings: StudioSettings): StudioSettings {
  return Object.freeze({ ...settings })
}

/** The render parameters preview and export both send; one mapping keeps them on one snapshot. */
export function previewParametersOf(settings: StudioSettings): PreviewParameters {
  return {
    cellSize: settings.cellSize,
    fontSize: settings.fontSize,
    threshold: settings.threshold,
    sampling: samplingForMode(settings.mode),
    mode: settings.mode,
    ramp: settings.ramp,
    foreground: settings.foreground,
    background: settings.background,
    colorMode: settings.colorMode,
    contrast: settings.contrast,
    invert: settings.invert,
  }
}

export function validateSettings(settings: StudioSettings): void {
  if (
    !Number.isInteger(settings.cellSize) ||
    settings.cellSize < MIN_CELL_SIZE ||
    settings.cellSize > MAX_CELL_SIZE
  ) {
    throw new RangeError(
      `Cell size must be an integer from ${MIN_CELL_SIZE} through ${MAX_CELL_SIZE}.`,
    )
  }
  if (
    !Number.isInteger(settings.fontSize) ||
    settings.fontSize < MIN_FONT_SIZE ||
    settings.fontSize > MAX_FONT_SIZE
  ) {
    throw new RangeError(
      `Font size must be an integer from ${MIN_FONT_SIZE} through ${MAX_FONT_SIZE}.`,
    )
  }
  if (
    !Number.isInteger(settings.threshold) ||
    settings.threshold < MIN_THRESHOLD ||
    settings.threshold > MAX_THRESHOLD
  ) {
    throw new RangeError(
      `Threshold must be an integer from ${MIN_THRESHOLD} through ${MAX_THRESHOLD}.`,
    )
  }
  if (!isRenderModeName(settings.mode)) {
    throw new RangeError('Render mode must be binary, ascii or block.')
  }
  const rampProblem = validateRamp(settings.ramp)
  if (rampProblem !== null) throw new RangeError(rampProblem)
  const foregroundProblem = validateColour(settings.foreground, 'foreground')
  if (foregroundProblem !== null) throw new RangeError(foregroundProblem)
  const backgroundProblem = validateColour(settings.background, 'background')
  if (backgroundProblem !== null) throw new RangeError(backgroundProblem)
  if (!isColorModeName(settings.colorMode)) throw new RangeError(COLOR_MODE_MESSAGE)
  const contrastProblem = validateContrast(settings.contrast)
  if (contrastProblem !== null) throw new RangeError(contrastProblem)
  if (typeof settings.invert !== 'boolean') throw new RangeError(INVERT_BOOLEAN_MESSAGE)
}
