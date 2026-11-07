import { describe, expect, it } from 'vitest'
import { createDemoImage, DEMO_IMAGE_HEIGHT, DEMO_IMAGE_WIDTH } from './demoImage'

/** FNV-1a 64 of the fixture bytes; the shell's Rust port checks the same constant. */
function fnv1a64(bytes: Uint8Array): string {
  let hash = 0xcbf29ce484222325n
  for (const byte of bytes) hash = ((hash ^ BigInt(byte)) * 0x100000001b3n) & 0xffffffffffffffffn
  return hash.toString(16).padStart(16, '0')
}

const SHARED_FNV1A64 = 'be5c6ff2f1f90429'

describe('demo image fixture', () => {
  it('hashes to the value the desktop shell reproduces', () => {
    expect(fnv1a64(new Uint8Array(createDemoImage().pixels))).toBe(SHARED_FNV1A64)
  })

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
