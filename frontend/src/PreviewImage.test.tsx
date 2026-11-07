import { describe, expect, it } from 'vitest'
import { isUpscaledOnDevice } from './PreviewImage'

const NATURAL = { width: 252, height: 192 }

describe('isUpscaledOnDevice', () => {
  it('is false when the displayed device pixels match the image pixels', () => {
    expect(isUpscaledOnDevice({ width: 252, height: 192 }, NATURAL, 1)).toBe(false)
  })

  it('turns on as soon as the device scale enlarges the image pixels', () => {
    expect(isUpscaledOnDevice({ width: 252, height: 192 }, NATURAL, 2)).toBe(true)
    expect(isUpscaledOnDevice({ width: 252, height: 192 }, NATURAL, 3)).toBe(true)
  })

  it('uses the contained scale, so the tighter axis decides', () => {
    expect(isUpscaledOnDevice({ width: 504, height: 192 }, NATURAL, 1)).toBe(false)
    expect(isUpscaledOnDevice({ width: 504, height: 384 }, NATURAL, 1)).toBe(true)
  })

  it('keeps downscaled previews smooth at any device scale', () => {
    expect(isUpscaledOnDevice({ width: 126, height: 96 }, NATURAL, 1)).toBe(false)
    expect(isUpscaledOnDevice({ width: 126, height: 96 }, NATURAL, 2)).toBe(false)
  })

  it('treats empty boxes and images as not upscaled', () => {
    expect(isUpscaledOnDevice({ width: 0, height: 192 }, NATURAL, 2)).toBe(false)
    expect(isUpscaledOnDevice({ width: 252, height: 192 }, { width: 0, height: 0 }, 2)).toBe(false)
  })
})
