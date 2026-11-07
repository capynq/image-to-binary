import { describe, expect, it } from 'vitest'
import { NATIVE_FRAME_HEADER_BYTES, parsePreviewFrame } from './nativeFrame'
import { ProcessingWorkerError } from './workerClient'

interface FrameParts {
  magic?: string
  version?: number
  generation?: number
  width?: number
  height?: number
  bodyLength?: number
}

/** Builds an I2BF body; overrides let each test break exactly one header rule. */
function frame(parts: FrameParts = {}): ArrayBuffer {
  const width = parts.width ?? 2
  const height = parts.height ?? 1
  const pixelCount = width * height * 4
  const length = parts.bodyLength ?? NATIVE_FRAME_HEADER_BYTES + pixelCount
  const bytes = new Uint8Array(length)
  const view = new DataView(bytes.buffer)
  const magic = parts.magic ?? 'I2BF'
  for (let index = 0; index < 4; index += 1) bytes[index] = magic.charCodeAt(index)
  view.setUint32(4, parts.version ?? 1, true)
  view.setBigUint64(8, BigInt(parts.generation ?? 7), true)
  view.setUint32(16, width, true)
  view.setUint32(20, height, true)
  for (let index = NATIVE_FRAME_HEADER_BYTES; index < length; index += 1) {
    bytes[index] = (index * 3) % 256
  }
  return bytes.buffer
}

function expectRejection(action: () => unknown, code: string, pattern: RegExp): void {
  try {
    action()
  } catch (error) {
    expect(error).toBeInstanceOf(ProcessingWorkerError)
    expect((error as ProcessingWorkerError).code).toBe(code)
    expect((error as ProcessingWorkerError).message).toMatch(pattern)
    return
  }
  throw new Error('The frame was accepted.')
}

describe('parsePreviewFrame', () => {
  it('returns the dimensions and the RGBA pixels that follow the header', () => {
    const body = frame({ width: 3, height: 2, generation: 9 })
    const parsed = parsePreviewFrame(body, 9)
    expect(parsed.width).toBe(3)
    expect(parsed.height).toBe(2)
    expect(new Uint8Array(parsed.pixels)).toEqual(new Uint8Array(body, NATIVE_FRAME_HEADER_BYTES))
  })

  it('rejects a wrong magic before reading anything else', () => {
    expectRejection(() => parsePreviewFrame(frame({ magic: 'XXXX' }), 7), 'internal', /magic/)
  })

  it('rejects an unsupported version', () => {
    expectRejection(() => parsePreviewFrame(frame({ version: 2 }), 7), 'internal', /version/)
  })

  it('rejects a body whose length is not 24 plus width times height times four', () => {
    expectRejection(
      () => parsePreviewFrame(frame({ bodyLength: NATIVE_FRAME_HEADER_BYTES + 7 }), 7),
      'internal',
      /length/,
    )
    expectRejection(
      () => parsePreviewFrame(frame({ bodyLength: NATIVE_FRAME_HEADER_BYTES + 9 }), 7),
      'internal',
      /length/,
    )
  })

  it('rejects a body shorter than the header', () => {
    expectRejection(() => parsePreviewFrame(new ArrayBuffer(10), 7), 'internal', /shorter/)
  })

  it('rejects a frame whose generation differs from the request', () => {
    expectRejection(
      () => parsePreviewFrame(frame({ generation: 8 }), 7),
      'stale_generation',
      /different generation/,
    )
  })

  it('rejects a body that is not an ArrayBuffer', () => {
    expectRejection(() => parsePreviewFrame({ not: 'bytes' }, 7), 'internal', /shorter/)
  })
})
