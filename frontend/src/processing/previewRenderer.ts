import type { PreviewPixels } from './workerClient'

export type PreviewRenderer = (pixels: PreviewPixels) => Promise<Blob>

/** Encodes RGBA preview pixels as a PNG Blob through a detached browser canvas. */
export async function renderPreviewPng(pixels: PreviewPixels): Promise<Blob> {
  const { width, height } = pixels
  if (pixels.pixels.byteLength !== width * height * 4) {
    throw new RangeError('Preview pixel buffer does not match its dimensions.')
  }
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  try {
    const context = canvas.getContext('2d')
    if (context === null) throw new Error('A 2D canvas context is unavailable.')
    context.putImageData(new ImageData(new Uint8ClampedArray(pixels.pixels), width, height), 0, 0)
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((blob) => {
        if (blob === null) {
          reject(new Error('PNG encoding failed.'))
        } else {
          resolve(blob)
        }
      }, 'image/png')
    })
  } finally {
    canvas.width = 0
    canvas.height = 0
  }
}
