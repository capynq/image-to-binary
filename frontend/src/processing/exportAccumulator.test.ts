import { describe, expect, it } from 'vitest'
import { createExportAccumulator, ExportLimitError, MAX_EXPORT_BYTES } from './exportAccumulator'
import { MAX_ENCODED_BYTES } from './protocol'

const bytes = (...values: number[]): ArrayBuffer => Uint8Array.from(values).buffer

describe('export accumulator', () => {
  it('uses the 256 MiB WASM encoded ceiling as its cumulative limit', () => {
    expect(MAX_EXPORT_BYTES).toBe(256 * 1024 * 1024)
    expect(MAX_EXPORT_BYTES).toBe(MAX_ENCODED_BYTES)
  })

  it('builds one image/png Blob with the exact bytes and length', async () => {
    const accumulator = createExportAccumulator()
    accumulator.accept(bytes(1, 2))
    accumulator.accept(bytes(3))
    const artifact = accumulator.finish({ chunkCount: 2, byteLength: 3 })
    expect(artifact.blob.type).toBe('image/png')
    expect(artifact.byteLength).toBe(3)
    expect(new Uint8Array(await artifact.blob.arrayBuffer())).toEqual(Uint8Array.of(1, 2, 3))
  })

  it('fails with a typed error and releases chunks once the limit would be exceeded', () => {
    const accumulator = createExportAccumulator({ limitBytes: 4 })
    accumulator.accept(bytes(1, 2, 3))
    expect(() => accumulator.accept(bytes(4, 5))).toThrow(ExportLimitError)
    expect(() => accumulator.accept(bytes(6))).toThrow('released')
    expect(() => accumulator.finish({ chunkCount: 1, byteLength: 3 })).toThrow('released')
  })

  it('rejects worker totals that disagree with accepted chunks and releases them', () => {
    const accumulator = createExportAccumulator()
    accumulator.accept(bytes(1))
    expect(() => accumulator.finish({ chunkCount: 1, byteLength: 2 })).toThrow('do not match')
    expect(() => accumulator.accept(bytes(2))).toThrow('released')
  })

  it('release is idempotent and leaves nothing to finish', () => {
    const accumulator = createExportAccumulator()
    accumulator.accept(bytes(1))
    accumulator.release()
    accumulator.release()
    expect(() => accumulator.finish({ chunkCount: 1, byteLength: 1 })).toThrow('released')
  })

  it('forwards progress fractions to its listener', () => {
    const fractions: number[] = []
    const accumulator = createExportAccumulator({
      onProgress: (fraction) => fractions.push(fraction),
    })
    accumulator.progress(0.25)
    accumulator.progress(1)
    expect(fractions).toEqual([0.25, 1])
  })
})

describe('export accumulator formats', () => {
  it('builds a text/plain UTF-8 Blob for the text format and records the format', async () => {
    const accumulator = createExportAccumulator({ format: 'text' })
    accumulator.accept(bytes(0x41, 0x0a))
    const artifact = accumulator.finish({ chunkCount: 1, byteLength: 2 })
    expect(artifact.kind).toBe('blob')
    expect(artifact.format).toBe('text')
    expect(artifact.blob.type).toBe('text/plain;charset=utf-8')
    expect(new Uint8Array(await artifact.blob.arrayBuffer())).toEqual(Uint8Array.of(0x41, 0x0a))
  })

  it('defaults to the PNG format and image/png type', () => {
    const artifact = createExportAccumulator().finish({ chunkCount: 0, byteLength: 0 })
    expect(artifact.format).toBe('png')
    expect(artifact.blob.type).toBe('image/png')
  })

  it('keeps a three-byte block character intact when its bytes are split across chunks', async () => {
    const block = '█'
    const encoded = [...new TextEncoder().encode(block)]
    expect(encoded).toEqual([0xe2, 0x96, 0x88])
    for (const split of [
      [[0xe2], [0x96, 0x88]],
      [[0xe2, 0x96], [0x88]],
      [[0xe2], [0x96], [0x88]],
    ]) {
      const accumulator = createExportAccumulator({ format: 'text' })
      for (const part of split) accumulator.accept(bytes(...part))
      const artifact = accumulator.finish({ chunkCount: split.length, byteLength: 3 })
      expect(new Uint8Array(await artifact.blob.arrayBuffer())).toEqual(Uint8Array.from(encoded))
      expect(await artifact.blob.text()).toBe(block)
    }
  })

  it('preserves a multi-line block grid byte for byte across one-byte chunks', async () => {
    const text = '█ ░\n  ▒▒\n'
    const encoded = new TextEncoder().encode(text)
    const accumulator = createExportAccumulator({ format: 'text' })
    for (const byte of encoded) accumulator.accept(bytes(byte))
    const artifact = accumulator.finish({ chunkCount: encoded.length, byteLength: encoded.length })
    expect(new Uint8Array(await artifact.blob.arrayBuffer())).toEqual(encoded)
  })
})
