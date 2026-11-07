import { isHeifDecodeReply } from './heifDecodeProtocol'
import type { HeifDecodeReply, HeifDecodeRequest } from './heifDecodeProtocol'
import { MAX_SOURCE_PIXELS, eventData, validateRgbaImage } from './protocol'
import { ProcessingWorkerError } from './workerClient'

/** Message surface of one dedicated decoder worker; `Worker` satisfies it. */
export interface HeifDecoderEndpoint {
  postMessage(message: unknown, transfer: Transferable[]): void
  addEventListener(type: string, listener: (event: Event) => void): void
  removeEventListener(type: string, listener: (event: Event) => void): void
  terminate(): void
}

export interface DecodedRgba {
  readonly width: number
  readonly height: number
  /** Fresh RGBA8 buffer owned by the receiver; it is transferred on to the processing worker. */
  readonly pixels: ArrayBuffer
}

export interface HeifDecodeJob {
  readonly result: Promise<DecodedRgba>
  /** Terminates the decoder and rejects `result` with `cancelled`; a no-op once settled. */
  cancel(): void
}

/**
 * Runs one decode in a worker obtained from `createEndpoint`. The worker is terminated exactly
 * once, when the job settles by reply, failure or `cancel()`. Events arriving after that are
 * never observed, because the listeners are detached before termination.
 *
 * There is no decode timer: a fixed deadline would fail large images on slow devices, so the job
 * waits for a reply, a failure or `cancel()`. `cancel()` always terminates the worker, including
 * mid-decode, so cancellation and replacement are the only ways to stop a long decode.
 */
export function startHeifDecode(
  file: File,
  createEndpoint: () => HeifDecoderEndpoint,
  maxPixels: number = MAX_SOURCE_PIXELS,
): HeifDecodeJob {
  let endpoint: HeifDecoderEndpoint | null = null
  let settled = false
  let resolveResult: (value: DecodedRgba) => void = () => undefined
  let rejectResult: (reason: Error) => void = () => undefined
  const result = new Promise<DecodedRgba>((resolve, reject) => {
    resolveResult = resolve
    rejectResult = reject
  })

  const release = (): void => {
    const worker = endpoint
    endpoint = null
    if (worker === null) return
    worker.removeEventListener('message', onMessage)
    worker.removeEventListener('error', onFailure)
    worker.removeEventListener('messageerror', onFailure)
    try {
      worker.terminate()
    } catch {}
  }

  const fail = (error: Error): void => {
    if (settled) return
    settled = true
    release()
    rejectResult(error)
  }

  const succeed = (decoded: DecodedRgba): void => {
    if (settled) return
    settled = true
    release()
    resolveResult(decoded)
  }

  function onFailure(): void {
    fail(decoderFailure())
  }

  function onMessage(event: Event): void {
    const data: unknown = eventData(event)
    if (!isHeifDecodeReply(data)) {
      fail(decoderFailure())
      return
    }
    settleWithReply(data)
  }

  const settleWithReply = (reply: HeifDecodeReply): void => {
    switch (reply.type) {
      case 'unavailable':
        fail(
          new ProcessingWorkerError(
            'codec_unavailable',
            'HEIC and AVIF are not available in this build. Export the image as JPEG, PNG or WebP and try again.',
          ),
        )
        return
      case 'failed':
        fail(failureForStatus(reply.status))
        return
      case 'decoded': {
        const invalid = validateRgbaImage(
          file.name,
          reply.width,
          reply.height,
          reply.pixels.byteLength,
        )
        if (invalid !== null) {
          fail(decoderFailure())
          return
        }
        succeed({ width: reply.width, height: reply.height, pixels: reply.pixels })
      }
    }
  }

  const request: HeifDecodeRequest = { type: 'decode', file, maxPixels }
  try {
    endpoint = createEndpoint()
    endpoint.addEventListener('message', onMessage)
    endpoint.addEventListener('error', onFailure)
    endpoint.addEventListener('messageerror', onFailure)
    endpoint.postMessage(request, [])
  } catch {
    fail(decoderFailure())
  }

  return {
    result,
    cancel(): void {
      fail(new ProcessingWorkerError('cancelled', 'HEIC and AVIF decoding was cancelled.'))
    },
  }
}

function failureForStatus(status: number): ProcessingWorkerError {
  switch (status) {
    case 1:
      return new ProcessingWorkerError(
        'unsupported_format',
        'This HEIC or AVIF file uses a feature the browser decoder does not support. Export it as JPEG or PNG and try again.',
      )
    case 2:
      return new ProcessingWorkerError(
        'decode_failed',
        'This HEIC or AVIF file is damaged or incomplete and could not be decoded. Export it again and choose the new file.',
      )
    case 3:
      return new ProcessingWorkerError(
        'resource_limit_exceeded',
        `This HEIC or AVIF image exceeds the decoder size limits, which allow up to ${MAX_SOURCE_PIXELS / 1_000_000} megapixels. Export a smaller image and try again.`,
      )
    case 4:
      return new ProcessingWorkerError(
        'resource_limit_exceeded',
        'The browser ran out of memory while decoding this HEIC or AVIF image. Close other tabs or export a smaller image and try again.',
      )
    default:
      return decoderFailure()
  }
}

function decoderFailure(): ProcessingWorkerError {
  return new ProcessingWorkerError(
    'decoder_failed',
    'The HEIC and AVIF decoder could not run for this image. Reload the page and try again, or export the image as JPEG or PNG.',
  )
}
