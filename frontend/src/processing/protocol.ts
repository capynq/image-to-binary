import { MAX_CONTRAST, MAX_RAMP_LENGTH, MIN_CONTRAST } from './bounds'

/**
 * Wire protocol between the processing client and the module worker.
 *
 * `requestId` is the scheduler-local generation of the job that produced the
 * request. It is the single identity source: it correlates replies, and the
 * worker also uses it as the strictly increasing WASM job generation. The UI
 * `JobContext.generation` never crosses this boundary.
 */
export const MAX_SOURCE_PIXELS = 50_000_000
export const MAX_RETAINED_SOURCES = 2
export const MAX_SOURCE_NAME_LENGTH = 256
export const MAX_PREVIEW_DIMENSION = 1600
export const MAX_ENCODED_BYTES = 256 * 1024 * 1024
/** Matches the default ASCII ramp of the shared argument contract, least ink first. */
export const DEFAULT_RAMP = ' .:-=+*#%@'
export const DEFAULT_FOREGROUND = '#00ff00'
export const DEFAULT_BACKGROUND = '#000000'

export type PreviewSampling = 'classic' | 'average'

export type ColourRole = 'foreground' | 'background'

/** `monochrome` paints cells in the foreground and background; `source` keeps the image colours. */
export type ColorModeName = 'monochrome' | 'source'

export const COLOR_MODE_NAMES: readonly ColorModeName[] = ['monochrome', 'source']

export function isColorModeName(value: unknown): value is ColorModeName {
  return COLOR_MODE_NAMES.some((name) => name === value)
}

export const FOREGROUND_MESSAGE = 'foreground must be a #RRGGBB colour'
export const BACKGROUND_MESSAGE = 'background must be a #RRGGBB colour'
export const COLOR_MODE_MESSAGE = 'colorMode must be "monochrome" or "source"'
export const CONTRAST_INTEGER_MESSAGE = 'contrast must be an integer'
export const CONTRAST_RANGE_MESSAGE = `contrast must be between ${MIN_CONTRAST} and ${MAX_CONTRAST}`
export const INVERT_BOOLEAN_MESSAGE = 'invert must be a boolean'

/** `png` is the default export; `text` is UTF-8 grid text over the same grid and settings. */
export type OutputKindName = 'png' | 'text'

export const OUTPUT_KIND_NAMES: readonly OutputKindName[] = ['png', 'text']

export const OUTPUT_KIND_MESSAGE = 'outputKind must be "png" or "text"'

export function isOutputKindName(value: unknown): value is OutputKindName {
  return OUTPUT_KIND_NAMES.some((name) => name === value)
}

const HEX_COLOUR = /^#[0-9a-fA-F]{6}$/

function isHexColour(text: string): boolean {
  return HEX_COLOUR.test(text)
}

/** Returns the readable reason a colour is rejected for its role, or `null` when it is `#RRGGBB`. */
export function validateColour(text: string, role: ColourRole): string | null {
  if (isHexColour(text)) return null
  return role === 'foreground' ? FOREGROUND_MESSAGE : BACKGROUND_MESSAGE
}

/** Returns the readable reason a contrast is rejected, or `null` for an integer within bounds. */
export function validateContrast(value: number): string | null {
  if (!Number.isInteger(value)) return CONTRAST_INTEGER_MESSAGE
  if (value < MIN_CONTRAST || value > MAX_CONTRAST) return CONTRAST_RANGE_MESSAGE
  return null
}

/** `binary` is the classic compatibility mode; `ascii` and `block` need average sampling. */
export type RenderModeName = 'binary' | 'ascii' | 'block'

export const RENDER_MODE_NAMES: readonly RenderModeName[] = ['binary', 'ascii', 'block']

export function isRenderModeName(value: unknown): value is RenderModeName {
  return RENDER_MODE_NAMES.some((name) => name === value)
}

/** The sampling a mode is rendered with: classic for binary, average for ascii and block. */
export function samplingForMode(mode: RenderModeName): PreviewSampling {
  return mode === 'binary' ? 'classic' : 'average'
}

/** Messages match the shared argument contract, so the page and the native shell read the same. */
export const RAMP_EMPTY_MESSAGE = 'ramp must contain at least one character'
export const RAMP_NOT_PRINTABLE_MESSAGE = 'ramp must contain only printable ASCII characters'
export const RAMP_TOO_LONG_MESSAGE = `ramp must be at most ${MAX_RAMP_LENGTH} characters`

/** Returns the readable reason a ramp is rejected, or `null` when it is a valid ramp. */
export function validateRamp(text: string): string | null {
  if (text.length === 0) return RAMP_EMPTY_MESSAGE
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index)
    if (code < 0x20 || code > 0x7e) return RAMP_NOT_PRINTABLE_MESSAGE
  }
  if (text.length > MAX_RAMP_LENGTH) return RAMP_TOO_LONG_MESSAGE
  return null
}

export interface PreviewParameters {
  readonly cellSize: number
  readonly fontSize: number
  readonly threshold: number
  readonly sampling: PreviewSampling
  readonly mode: RenderModeName
  /** Printable ASCII ramp; read only when `mode` is `ascii`. */
  readonly ramp: string
  /** `#RRGGBB` cell colour used by monochrome output. */
  readonly foreground: string
  /** `#RRGGBB` backdrop colour used by monochrome output. */
  readonly background: string
  readonly colorMode: ColorModeName
  /** Integer in [MIN_CONTRAST, MAX_CONTRAST]. */
  readonly contrast: number
  readonly invert: boolean
}

export interface RequestIdentity {
  readonly requestId: number
  readonly operationId: number
}

/** Transfers ownership of `pixels` (normalized RGBA8, width * height * 4 bytes) to the worker. */
export interface ImportRequest extends RequestIdentity {
  readonly type: 'import'
  readonly name: string
  readonly width: number
  readonly height: number
  readonly pixels: ArrayBuffer
}

export interface PreviewRequest extends RequestIdentity {
  readonly type: 'preview'
  readonly sourceId: number
  readonly parameters: PreviewParameters
}

/** Transfers ownership of `bytes` (an encoded BMP, GIF, JPEG, PNG, TIFF or WebP file) to the worker. */
export interface ImportEncodedRequest extends RequestIdentity {
  readonly type: 'importEncoded'
  readonly name: string
  readonly bytes: ArrayBuffer
}

/** Requests the bounded original-image preview of a retained source; answered with `previewed`. */
export interface OriginalPreviewRequest extends RequestIdentity {
  readonly type: 'originalPreview'
  readonly sourceId: number
}

/** Starts an export of a retained source in `outputKind`; answered by ordered `exportChunk` messages and one `exported`. */
export interface ExportRequest extends RequestIdentity {
  readonly type: 'export'
  readonly sourceId: number
  readonly parameters: PreviewParameters
  readonly outputKind: OutputKindName
}

export interface CancelRequest extends RequestIdentity {
  readonly type: 'cancel'
}

export interface ReleaseSourceRequest {
  readonly type: 'releaseSource'
  readonly sourceId: number
}

export type WorkerRequest =
  | ImportRequest
  | ImportEncodedRequest
  | PreviewRequest
  | OriginalPreviewRequest
  | ExportRequest
  | CancelRequest
  | ReleaseSourceRequest

export interface ImportedReply extends RequestIdentity {
  readonly type: 'imported'
  readonly sourceId: number
  readonly name: string
  readonly width: number
  readonly height: number
}

/**
 * Carries a fresh RGBA8 buffer owned by the receiver; never a WASM memory view.
 * Geometry is bounded by `validatePreviewFrame`.
 */
export interface PreviewedReply extends RequestIdentity {
  readonly type: 'previewed'
  readonly width: number
  readonly height: number
  readonly pixels: ArrayBuffer
}

/** One ordered, worker-owned slice of the export bytes; `sequence` starts at 0 and never repeats. */
export interface ExportChunkReply extends RequestIdentity {
  readonly type: 'exportChunk'
  readonly sequence: number
  readonly bytes: ArrayBuffer
}

/** Monotonic encoding progress in the engine's unit; `completed` never exceeds `total`. */
export interface ExportProgressReply extends RequestIdentity {
  readonly type: 'exportProgress'
  readonly completed: number
  readonly total: number
}

/** Terminal success: totals the client compares with the chunks it received. */
export interface ExportedReply extends RequestIdentity {
  readonly type: 'exported'
  readonly chunkCount: number
  readonly byteLength: number
}

export interface CancelledReply extends RequestIdentity {
  readonly type: 'cancelled'
}

export interface FailedReply extends RequestIdentity {
  readonly type: 'failed'
  readonly code: string
  readonly message: string
}

/** The worker cannot continue; the client fails the whole session. */
export interface FatalReply {
  readonly type: 'fatal'
  readonly message: string
}

export type WorkerReply =
  | ImportedReply
  | PreviewedReply
  | ExportChunkReply
  | ExportProgressReply
  | ExportedReply
  | CancelledReply
  | FailedReply
  | FatalReply

export interface ValidationFailure {
  readonly code: 'invalid_argument' | 'invalid_dimensions' | 'resource_limit_exceeded'
  readonly message: string
}

/** Validates name and dimensions; shared by requests, imported replies and pixel checks. */
export function validateImageMetadata(
  name: string,
  width: number,
  height: number,
): ValidationFailure | null {
  if (name.length === 0 || name.length > MAX_SOURCE_NAME_LENGTH) {
    return {
      code: 'invalid_argument',
      message: `Source name must be 1 to ${MAX_SOURCE_NAME_LENGTH} characters.`,
    }
  }
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1) {
    return { code: 'invalid_dimensions', message: 'Source dimensions must be positive integers.' }
  }
  if (width * height > MAX_SOURCE_PIXELS) {
    return {
      code: 'resource_limit_exceeded',
      message: `Source exceeds the ${MAX_SOURCE_PIXELS.toLocaleString('en-US')} pixel limit.`,
    }
  }
  return null
}

/** Shared by the client (before transfer) and the worker (before the WASM copy). */
export function validateRgbaImage(
  name: string,
  width: number,
  height: number,
  byteLength: number,
): ValidationFailure | null {
  const invalid = validateImageMetadata(name, width, height)
  if (invalid !== null) return invalid
  if (byteLength !== width * height * 4) {
    return {
      code: 'invalid_argument',
      message: 'Pixel buffer length must equal width × height × 4.',
    }
  }
  return null
}

/** Validates an encoded import before any byte is copied into the engine. */
export function validateEncodedImport(name: string, byteLength: number): ValidationFailure | null {
  if (name.length === 0 || name.length > MAX_SOURCE_NAME_LENGTH) {
    return {
      code: 'invalid_argument',
      message: `Source name must be 1 to ${MAX_SOURCE_NAME_LENGTH} characters.`,
    }
  }
  if (!Number.isSafeInteger(byteLength) || byteLength < 1) {
    return { code: 'invalid_argument', message: 'Encoded image bytes must not be empty.' }
  }
  if (byteLength > MAX_ENCODED_BYTES) {
    return {
      code: 'resource_limit_exceeded',
      message: `Encoded image exceeds the ${MAX_ENCODED_BYTES.toLocaleString('en-US')} byte limit.`,
    }
  }
  return null
}

/** Bounds a preview reply to MAX_PREVIEW_DIMENSION per side with an exact RGBA length. */
export function validatePreviewFrame(
  width: number,
  height: number,
  byteLength: number,
): ValidationFailure | null {
  if (
    Number.isSafeInteger(width) &&
    Number.isSafeInteger(height) &&
    (width > MAX_PREVIEW_DIMENSION || height > MAX_PREVIEW_DIMENSION)
  ) {
    return {
      code: 'resource_limit_exceeded',
      message: `Preview frames are limited to ${MAX_PREVIEW_DIMENSION} pixels per side.`,
    }
  }
  return validateRgbaImage('preview', width, height, byteLength)
}

export function isArrayBuffer(value: unknown): value is ArrayBuffer {
  return Object.prototype.toString.call(value) === '[object ArrayBuffer]'
}

export function eventData(event: Event): unknown {
  return 'data' in event ? event.data : undefined
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}

function isRequestId(value: unknown): value is number {
  return isCount(value) && value >= 1
}

function isSourceId(value: unknown): value is number {
  return isCount(value) && value >= 1
}

function hasSourceMetadata(value: Record<string, unknown>): boolean {
  const { name, width, height } = value
  return (
    typeof name === 'string' &&
    typeof width === 'number' &&
    typeof height === 'number' &&
    validateImageMetadata(name, width, height) === null
  )
}

function hasPreviewFrame(value: Record<string, unknown>): boolean {
  const { width, height, pixels } = value
  return (
    typeof width === 'number' &&
    typeof height === 'number' &&
    isArrayBuffer(pixels) &&
    validatePreviewFrame(width, height, pixels.byteLength) === null
  )
}

function hasIdentity(value: Record<string, unknown>): boolean {
  return isRequestId(value.requestId) && isCount(value.operationId)
}

function isContrast(value: unknown): value is number {
  return typeof value === 'number' && validateContrast(value) === null
}

function isPreviewParameters(value: unknown): value is PreviewParameters {
  return (
    isRecord(value) &&
    isCount(value.cellSize) &&
    isCount(value.fontSize) &&
    isCount(value.threshold) &&
    (value.sampling === 'classic' || value.sampling === 'average') &&
    isRenderModeName(value.mode) &&
    typeof value.ramp === 'string' &&
    typeof value.foreground === 'string' &&
    isHexColour(value.foreground) &&
    typeof value.background === 'string' &&
    isHexColour(value.background) &&
    isColorModeName(value.colorMode) &&
    isContrast(value.contrast) &&
    typeof value.invert === 'boolean'
  )
}

export function isWorkerRequest(value: unknown): value is WorkerRequest {
  if (!isRecord(value)) return false
  switch (value.type) {
    case 'import':
      return (
        hasIdentity(value) &&
        typeof value.name === 'string' &&
        isCount(value.width) &&
        isCount(value.height) &&
        isArrayBuffer(value.pixels)
      )
    case 'preview':
      return (
        hasIdentity(value) &&
        isCount(value.sourceId) &&
        isPreviewParameters(value.parameters) &&
        !('outputKind' in value)
      )
    case 'export':
      return (
        hasIdentity(value) &&
        isCount(value.sourceId) &&
        isPreviewParameters(value.parameters) &&
        isOutputKindName(value.outputKind)
      )
    case 'importEncoded':
      return hasIdentity(value) && typeof value.name === 'string' && isArrayBuffer(value.bytes)
    case 'originalPreview':
      return hasIdentity(value) && isSourceId(value.sourceId)
    case 'cancel':
      return hasIdentity(value)
    case 'releaseSource':
      return isCount(value.sourceId)
    default:
      return false
  }
}

export function isWorkerReply(value: unknown): value is WorkerReply {
  if (!isRecord(value)) return false
  switch (value.type) {
    case 'imported':
      return hasIdentity(value) && isSourceId(value.sourceId) && hasSourceMetadata(value)
    case 'previewed':
      return hasIdentity(value) && hasPreviewFrame(value)
    case 'exportChunk':
      return (
        hasIdentity(value) &&
        isCount(value.sequence) &&
        isArrayBuffer(value.bytes) &&
        value.bytes.byteLength > 0 &&
        value.bytes.byteLength <= MAX_ENCODED_BYTES
      )
    case 'exportProgress':
      return (
        hasIdentity(value) &&
        isCount(value.completed) &&
        isCount(value.total) &&
        value.completed <= value.total
      )
    case 'exported':
      return (
        hasIdentity(value) &&
        isCount(value.chunkCount) &&
        isCount(value.byteLength) &&
        value.byteLength <= MAX_ENCODED_BYTES
      )
    case 'cancelled':
      return hasIdentity(value)
    case 'failed':
      return (
        hasIdentity(value) && typeof value.code === 'string' && typeof value.message === 'string'
      )
    case 'fatal':
      return typeof value.message === 'string'
    default:
      return false
  }
}
