import { describe, expect, it } from 'vitest'
import createUnavailableGlue from './heif/unavailableGlue'
import { HEIF_CODEC_UNAVAILABLE, isHeifDecodeRequest } from './heifDecodeProtocol'
import { decodeWithModule, runHeifDecode } from './heifDecoderCore'
import type { HeifModule } from './heifDecoderCore'
import { MAX_ENCODED_BYTES } from './protocol'

class FakeHeifModule implements HeifModule {
  readonly HEAPU8 = new Uint8Array(new ArrayBuffer(128))
  status = 0
  width = 2
  height = 1
  mallocResult = 8
  released = 0
  freed = 0
  readonly pixelPointer = 64

  constructor(rgba: number[] = [1, 2, 3, 4, 5, 6, 7, 8]) {
    this.HEAPU8.set(rgba, this.pixelPointer)
  }

  _malloc(): number {
    return this.mallocResult
  }

  _free(): void {
    this.freed += 1
  }

  _hwc_decode(): number {
    if (this.status !== 0) {
      this.width = 0
      this.height = 0
    }
    return this.status
  }

  _hwc_width(): number {
    return this.width
  }

  _hwc_height(): number {
    return this.height
  }

  _hwc_pixels(): number {
    return this.pixelPointer
  }

  _hwc_release(): void {
    this.released += 1
  }
}

const file = new File([new Uint8Array([9, 9, 9])], 'shot.avif', { type: 'image/avif' })

describe('decodeWithModule', () => {
  it('copies RGBA into a fresh buffer and releases the decoder and input on success', () => {
    const module = new FakeHeifModule()
    const reply = decodeWithModule(module, new Uint8Array(3), 50_000_000)
    expect(reply.type).toBe('decoded')
    if (reply.type !== 'decoded') return
    expect(reply.width).toBe(2)
    expect(reply.height).toBe(1)
    expect(Array.from(new Uint8Array(reply.pixels))).toEqual([1, 2, 3, 4, 5, 6, 7, 8])
    expect(reply.pixels).not.toBe(module.HEAPU8.buffer)

    module.HEAPU8.fill(0)
    expect(Array.from(new Uint8Array(reply.pixels))).toEqual([1, 2, 3, 4, 5, 6, 7, 8])
    expect(module.released).toBe(1)
    expect(module.freed).toBe(1)
  })

  it.each([1, 2, 3, 4])('returns status %i as failed and still releases and frees', (status) => {
    const module = new FakeHeifModule()
    module.status = status
    expect(decodeWithModule(module, new Uint8Array(3), 50_000_000)).toEqual({
      type: 'failed',
      status,
    })
    expect(module.released).toBe(1)
    expect(module.freed).toBe(1)
  })

  it('reports out of memory without touching the decoder when allocation fails', () => {
    const module = new FakeHeifModule()
    module.mallocResult = 0
    expect(decodeWithModule(module, new Uint8Array(3), 50_000_000)).toEqual({
      type: 'failed',
      status: 4,
    })
    expect(module.released).toBe(0)
    expect(module.freed).toBe(0)
  })
})

describe('runHeifDecode', () => {
  const request = { type: 'decode' as const, file, maxPixels: 50_000_000 }

  it('decodes the file through the loaded module', async () => {
    const reply = await runHeifDecode(async () => new FakeHeifModule(), request)
    expect(reply.type).toBe('decoded')
  })

  it('reports the typed unavailable reply when the glue is the unavailable stub', async () => {
    await expect(runHeifDecode(createUnavailableGlue, request)).resolves.toEqual({
      type: 'unavailable',
    })
  })

  it('refuses an oversized encoded file as status 3 before the decoder loads', async () => {
    const oversized = new File([new Uint8Array([1])], 'big.heic', { type: 'image/heic' })
    Object.defineProperty(oversized, 'size', { value: MAX_ENCODED_BYTES + 1 })
    let loads = 0
    const reply = await runHeifDecode(
      async () => {
        loads += 1
        return new FakeHeifModule()
      },
      { type: 'decode', file: oversized, maxPixels: 50_000_000 },
    )
    expect(reply).toEqual({ type: 'failed', status: 3 })
    expect(loads).toBe(0)
  })

  it('refuses an empty encoded file as status 2 before the decoder loads', async () => {
    const empty = new File([], 'empty.heic', { type: 'image/heic' })
    let loads = 0
    const reply = await runHeifDecode(
      async () => {
        loads += 1
        return new FakeHeifModule()
      },
      { type: 'decode', file: empty, maxPixels: 50_000_000 },
    )
    expect(reply).toEqual({ type: 'failed', status: 2 })
    expect(loads).toBe(0)
  })

  it('reports a generic load failure as status 0 rather than rejecting', async () => {
    const reply = await runHeifDecode(() => Promise.reject(new Error('wasm fetch failed')), request)
    expect(reply).toEqual({ type: 'failed', status: 0 })
  })

  it('names the unavailable error exactly as the stub rejects with it', async () => {
    const error = await createUnavailableGlue().catch((reason: unknown) => reason)
    expect(error).toBeInstanceOf(Error)
    expect((error as Error).name).toBe(HEIF_CODEC_UNAVAILABLE)
    expect((error as Error).message).toBe('HEIC and AVIF decoding is not included in this build.')
  })
})

describe('isHeifDecodeRequest', () => {
  it('accepts only a decode request that carries a File and a positive ceiling', () => {
    expect(isHeifDecodeRequest({ type: 'decode', file, maxPixels: 1 })).toBe(true)
    expect(isHeifDecodeRequest({ type: 'decode', file, maxPixels: 0 })).toBe(false)
    expect(isHeifDecodeRequest({ type: 'decode', file: new Uint8Array(1), maxPixels: 1 })).toBe(
      false,
    )
    expect(isHeifDecodeRequest({ type: 'import', file, maxPixels: 1 })).toBe(false)
    expect(isHeifDecodeRequest(null)).toBe(false)
  })
})
