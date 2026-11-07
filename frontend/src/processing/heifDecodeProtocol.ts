import { isArrayBuffer } from './protocol'

/** Error name that the unavailable-build glue rejects with; the decoder worker reports it as `unavailable`. */
export const HEIF_CODEC_UNAVAILABLE = 'HeifCodecUnavailableError'

/**
 * Posted to a dedicated decoder worker. The `File` is structured-cloned by reference, so no
 * encoded bytes cross the main thread; `maxPixels` is the shared source ceiling.
 */
export interface HeifDecodeRequest {
  readonly type: 'decode'
  readonly file: File
  readonly maxPixels: number
}

/**
 * Terminal reply of one decode. `failed.status` is the C glue status (1 unsupported, 2 invalid,
 * 3 limit, 4 out of memory) or 0 for a worker-side failure. `decoded.pixels` is a fresh RGBA8
 * buffer owned by the receiver.
 */
export type HeifDecodeReply =
  | {
      readonly type: 'decoded'
      readonly width: number
      readonly height: number
      readonly pixels: ArrayBuffer
    }
  | { readonly type: 'failed'; readonly status: number }
  | { readonly type: 'unavailable' }

export function isHeifDecodeRequest(value: unknown): value is HeifDecodeRequest {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Partial<Record<'type' | 'file' | 'maxPixels', unknown>>
  return (
    candidate.type === 'decode' &&
    typeof File === 'function' &&
    candidate.file instanceof File &&
    isPositiveInteger(candidate.maxPixels)
  )
}

export function isHeifDecodeReply(value: unknown): value is HeifDecodeReply {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Partial<
    Record<'type' | 'width' | 'height' | 'pixels' | 'status', unknown>
  >
  switch (candidate.type) {
    case 'decoded':
      return (
        isPositiveInteger(candidate.width) &&
        isPositiveInteger(candidate.height) &&
        isArrayBuffer(candidate.pixels)
      )
    case 'failed':
      return typeof candidate.status === 'number' && Number.isSafeInteger(candidate.status)
    case 'unavailable':
      return true
    default:
      return false
  }
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 1
}
