import { describe, expect, it } from 'vitest'
import { createDemoImage, DEMO_IMAGE_HEIGHT, DEMO_IMAGE_WIDTH } from './demoImage'

describe('demo image fixture', () => {
  it('is deterministic, opaque and nonuniform', () => {
    const first = createDemoImage()
    const second = createDemoImage()
    expect(first.width).toBe(DEMO_IMAGE_WIDTH)
    expect(first.height).toBe(DEMO_IMAGE_HEIGHT)
    expect(first.pixels.byteLength).toBe(DEMO_IMAGE_WIDTH * DEMO_IMAGE_HEIGHT * 4)
    const a = new Uint8Array(first.pixels)
    expect(a).toEqual(new Uint8Array(second.pixels))
    expect(new Set(a.filter((_, index) => index % 4 === 0)).size).toBeGreaterThan(64)
    expect(a.every((value, index) => index % 4 !== 3 || value === 255)).toBe(true)
  })
})
