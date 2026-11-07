import { ProcessingWorkerError } from './workerClient'

/** Header of the I2BF preview body the shell returns as raw bytes; see `crates/desktop/src/processing/frame.rs`. */
export const NATIVE_FRAME_HEADER_BYTES = 24
const MAGIC = 'I2BF'
const VERSION = 1

export interface NativeFrame {
  readonly width: number
  readonly height: number
  readonly pixels: ArrayBuffer
}

/**
 * Validates a preview body and returns its RGBA pixels. Magic, version, length and the
 * request's generation are all checked before any pixel is exposed, so a mismatched frame is a
 * typed failure, never a displayed image.
 */
export function parsePreviewFrame(body: unknown, generation: number): NativeFrame {
  if (!(body instanceof ArrayBuffer) || body.byteLength < NATIVE_FRAME_HEADER_BYTES) {
    throw invalidFrame('the preview frame is shorter than its header')
  }
  const view = new DataView(body)
  if (new TextDecoder().decode(new Uint8Array(body, 0, 4)) !== MAGIC) {
    throw invalidFrame('the preview frame has the wrong magic')
  }
  if (view.getUint32(4, true) !== VERSION) {
    throw invalidFrame('the preview frame has an unsupported version')
  }
  const width = view.getUint32(16, true)
  const height = view.getUint32(20, true)
  const expected = BigInt(NATIVE_FRAME_HEADER_BYTES) + BigInt(width) * BigInt(height) * 4n
  if (expected !== BigInt(body.byteLength)) {
    throw invalidFrame('the preview frame length does not match its dimensions')
  }
  if (view.getBigUint64(8, true) !== BigInt(generation)) {
    throw new ProcessingWorkerError(
      'stale_generation',
      'the preview frame belongs to a different generation',
    )
  }
  return { width, height, pixels: body.slice(NATIVE_FRAME_HEADER_BYTES) }
}

function invalidFrame(detail: string): ProcessingWorkerError {
  return new ProcessingWorkerError('internal', `Invalid preview frame: ${detail}.`)
}
