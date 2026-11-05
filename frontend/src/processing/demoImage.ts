import type { RgbaImport } from './workerClient'

export const DEMO_IMAGE_WIDTH = 256
export const DEMO_IMAGE_HEIGHT = 192

/** Builds a deterministic, nonuniform RGBA8 fixture: tiles, a ring and colour ramps. */
export function createDemoImage(): RgbaImport {
  const width = DEMO_IMAGE_WIDTH
  const height = DEMO_IMAGE_HEIGHT
  const data = new Uint8Array(width * height * 4)
  for (let y = 0; y < height; y += 1) {
    const rampY = Math.floor((y * 255) / (height - 1))
    for (let x = 0; x < width; x += 1) {
      const rampX = Math.floor((x * 255) / (width - 1))
      const dx = x - width / 2
      const dy = y - height / 2
      const ring = Math.abs(Math.sqrt(dx * dx + dy * dy) - 72) < 9
      const tile = ((x >> 5) + (y >> 5)) & 1
      const luma = ring ? 16 : tile === 0 ? rampX : 255 - rampX
      const offset = (y * width + x) * 4
      data[offset] = luma
      data[offset + 1] = ring ? 64 : luma
      data[offset + 2] = rampY
      data[offset + 3] = 255
    }
  }
  return { name: 'Demo fixture', width, height, pixels: data.buffer }
}
