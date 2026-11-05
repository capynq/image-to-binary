import { describe, expect, it } from 'vitest'
import { isWorkerReply, isWorkerRequest, validatePreviewFrame, validateRgbaImage } from './protocol'

const parameters = { cellSize: 12, fontSize: 16, threshold: 128, sampling: 'classic' }

describe('protocol guards', () => {
  it('accepts well-formed requests', () => {
    expect(
      isWorkerRequest({
        type: 'import',
        requestId: 1,
        operationId: 1,
        name: 'n',
        width: 1,
        height: 1,
        pixels: new ArrayBuffer(4),
      }),
    ).toBe(true)
    expect(
      isWorkerRequest({ type: 'preview', requestId: 2, operationId: 1, sourceId: 1, parameters }),
    ).toBe(true)
    expect(isWorkerRequest({ type: 'cancel', requestId: 2, operationId: 1 })).toBe(true)
    expect(isWorkerRequest({ type: 'releaseSource', sourceId: 3 })).toBe(true)
  })

  it.each([
    null,
    'import',
    {},
    { type: 'unknown' },
    { type: 'cancel', requestId: 0, operationId: 1 },
    { type: 'cancel', requestId: 1.5, operationId: 1 },
    { type: 'cancel', requestId: 1, operationId: -1 },
    { type: 'releaseSource', sourceId: '1' },
    { type: 'preview', requestId: 1, operationId: 1, sourceId: 1, parameters: { cellSize: 1 } },
    {
      type: 'import',
      requestId: 1,
      operationId: 1,
      name: 'n',
      width: 1,
      height: 1,
      pixels: new Uint8Array(4),
    },
  ])('rejects malformed request %#', (value) => {
    expect(isWorkerRequest(value)).toBe(false)
  })

  it('validates replies including pixel buffers', () => {
    expect(
      isWorkerReply({
        type: 'previewed',
        requestId: 1,
        operationId: 1,
        width: 1,
        height: 1,
        pixels: new ArrayBuffer(4),
      }),
    ).toBe(true)
    expect(
      isWorkerReply({
        type: 'previewed',
        requestId: 1,
        operationId: 1,
        width: 1,
        height: 1,
        pixels: [1, 2, 3, 4],
      }),
    ).toBe(false)
    expect(isWorkerReply({ type: 'fatal', message: 'x' })).toBe(true)
    expect(isWorkerReply({ type: 'failed', requestId: 1, operationId: 1, code: 'c' })).toBe(false)
  })

  it('accepts a well-formed imported reply', () => {
    expect(
      isWorkerReply({
        type: 'imported',
        requestId: 1,
        operationId: 1,
        sourceId: 1,
        name: 'n',
        width: 2,
        height: 1,
      }),
    ).toBe(true)
  })

  it.each([
    ['source id 0', { sourceId: 0, name: 'n', width: 2, height: 1 }],
    ['fractional source id', { sourceId: 1.5, name: 'n', width: 2, height: 1 }],
    ['empty name', { sourceId: 1, name: '', width: 2, height: 1 }],
    ['overlong name', { sourceId: 1, name: 'x'.repeat(257), width: 2, height: 1 }],
    ['zero width', { sourceId: 1, name: 'n', width: 0, height: 1 }],
    ['zero height', { sourceId: 1, name: 'n', width: 1, height: 0 }],
    ['over 50MP', { sourceId: 1, name: 'n', width: 10_000, height: 10_000 }],
  ])('rejects an imported reply with %s', (_name, fields) => {
    expect(isWorkerReply({ type: 'imported', requestId: 1, operationId: 1, ...fields })).toBe(false)
  })

  it.each([
    ['oversized width', 1601, 1, 1601 * 4],
    ['oversized height', 1, 1601, 1601 * 4],
    ['zero width', 0, 1, 0],
    ['short buffer', 2, 1, 7],
    ['long buffer', 2, 1, 9],
  ])('rejects a previewed reply with %s', (_name, width, height, bytes) => {
    expect(
      isWorkerReply({
        type: 'previewed',
        requestId: 1,
        operationId: 1,
        width,
        height,
        pixels: new ArrayBuffer(bytes),
      }),
    ).toBe(false)
  })

  it('bounds preview frames to 1600 pixels per side with an exact length', () => {
    expect(validatePreviewFrame(1600, 1, 6400)).toBeNull()
    expect(validatePreviewFrame(1, 1600, 6400)).toBeNull()
    expect(validatePreviewFrame(1601, 1, 6404)?.code).toBe('resource_limit_exceeded')
    expect(validatePreviewFrame(2, 1, 7)?.code).toBe('invalid_argument')
    expect(validatePreviewFrame(0, 1, 0)?.code).toBe('invalid_dimensions')
  })

  it('validates image dimensions, limits and length', () => {
    expect(validateRgbaImage('n', 2, 1, 8)).toBeNull()
    expect(validateRgbaImage('', 2, 1, 8)?.code).toBe('invalid_argument')
    expect(validateRgbaImage('n', 0, 1, 0)?.code).toBe('invalid_dimensions')
    expect(validateRgbaImage('n', 10_000, 10_000, 8)?.code).toBe('resource_limit_exceeded')
    expect(validateRgbaImage('n', 2, 1, 7)?.code).toBe('invalid_argument')
  })
})
