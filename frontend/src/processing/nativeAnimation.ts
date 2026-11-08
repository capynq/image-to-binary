import { ANIMATION_BUDGET_MESSAGE, ANIMATION_PREVIEW_BUDGET_BYTES } from './browserAdapter'
import type { PreviewArtifact, UrlApi } from './types'
import type { PreviewPixels } from './workerClient'
import { ProcessingWorkerError } from './workerClient'

export interface NativeAnimationOptions {
  readonly frameCount: number
  readonly startFrame: number
  readonly isCancelled: () => boolean
  readonly onProgress: ((fraction: number) => void) | undefined
  /** Fetches one frame's pixels from the shell, already validated for this generation. */
  readonly renderFrame: (frame: number) => Promise<PreviewPixels>
  readonly encode: (pixels: PreviewPixels) => Promise<Blob>
  readonly urlApi: UrlApi
}

interface EncodedFrame {
  readonly url: string
  readonly width: number
  readonly height: number
  readonly byteLength: number
}

/**
 * Renders every frame of an animation in wrap-around order from `startFrame`, mirroring the
 * browser adapter. The cancel flag is read before each frame and after the last one. A cancel or
 * a failure revokes every URL already made. Encoded PNG bytes across all frames are capped at
 * `ANIMATION_PREVIEW_BUDGET_BYTES`, checked before a frame's URL is created.
 */
export async function renderNativeAnimation(
  options: NativeAnimationOptions,
): Promise<PreviewArtifact> {
  const { frameCount, startFrame, isCancelled, onProgress, renderFrame, encode, urlApi } = options
  const encoded: Array<EncodedFrame & { readonly revoke: () => void }> = []
  let retainedBytes = 0
  try {
    for (let step = 0; step < frameCount; step += 1) {
      assertNotCancelled(isCancelled)
      const frame = (startFrame + step) % frameCount
      const pixels = await renderFrame(frame)
      const blob = await encode(pixels)
      assertNotCancelled(isCancelled)
      if (retainedBytes + blob.size > ANIMATION_PREVIEW_BUDGET_BYTES) {
        throw new RangeError(ANIMATION_BUDGET_MESSAGE)
      }
      const url = urlApi.createObjectURL(blob)
      let revoked = false
      encoded[frame] = {
        url,
        width: pixels.width,
        height: pixels.height,
        byteLength: blob.size,
        revoke: () => {
          if (!revoked) {
            revoked = true
            urlApi.revokeObjectURL(url)
          }
        },
      }
      retainedBytes += blob.size
      onProgress?.((step + 1) / frameCount)
    }
    assertNotCancelled(isCancelled)
  } catch (error) {
    encoded.forEach((frame) => frame.revoke())
    throw isCancelled() ? cancelledError() : error
  }
  const start = encoded[startFrame]
  if (start === undefined) throw new Error('The animation preview lost its start frame.')
  return {
    url: start.url,
    width: start.width,
    height: start.height,
    frames: Object.freeze(encoded.map((frame) => frame.url)),
    dispose(): void {
      encoded.forEach((frame) => frame.revoke())
    },
  }
}

function assertNotCancelled(isCancelled: () => boolean): void {
  if (isCancelled()) throw cancelledError()
}

function cancelledError(): ProcessingWorkerError {
  return new ProcessingWorkerError('cancelled', 'Animation preview was cancelled.')
}
