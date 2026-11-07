import { describe, expect, it } from 'vitest'
import {
  DEFAULT_RAMP,
  MAX_ENCODED_BYTES,
  MAX_RAMP_LENGTH,
  RAMP_EMPTY_MESSAGE,
  RAMP_NOT_PRINTABLE_MESSAGE,
  RAMP_TOO_LONG_MESSAGE,
  samplingForMode,
  validateRamp,
  isWorkerReply,
  isWorkerRequest,
  validateEncodedImport,
  validatePreviewFrame,
  validateRgbaImage,
} from './protocol'

const parameters = {
  cellSize: 12,
  fontSize: 16,
  threshold: 128,
  sampling: 'classic',
  mode: 'binary',
  ramp: DEFAULT_RAMP,
} as const

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

describe('encoded import and original preview protocol', () => {
  it('accepts well-formed encoded import and original preview requests', () => {
    expect(
      isWorkerRequest({
        type: 'importEncoded',
        requestId: 1,
        operationId: 1,
        name: 'photo.png',
        bytes: new ArrayBuffer(3),
      }),
    ).toBe(true)
    expect(
      isWorkerRequest({ type: 'originalPreview', requestId: 2, operationId: 1, sourceId: 1 }),
    ).toBe(true)
  })

  it.each([
    [
      'typed array bytes',
      { type: 'importEncoded', requestId: 1, operationId: 1, name: 'n', bytes: new Uint8Array(3) },
    ],
    ['missing bytes', { type: 'importEncoded', requestId: 1, operationId: 1, name: 'n' }],
    [
      'non-string name',
      { type: 'importEncoded', requestId: 1, operationId: 1, name: 5, bytes: new ArrayBuffer(3) },
    ],
    [
      'zero request id',
      { type: 'importEncoded', requestId: 0, operationId: 1, name: 'n', bytes: new ArrayBuffer(3) },
    ],
    [
      'missing operation id',
      { type: 'importEncoded', requestId: 1, name: 'n', bytes: new ArrayBuffer(3) },
    ],
    ['missing source id', { type: 'originalPreview', requestId: 1, operationId: 1 }],
    ['zero source id', { type: 'originalPreview', requestId: 1, operationId: 1, sourceId: 0 }],
    [
      'fractional source id',
      { type: 'originalPreview', requestId: 1, operationId: 1, sourceId: 1.5 },
    ],
    [
      'zero request id original',
      { type: 'originalPreview', requestId: 0, operationId: 1, sourceId: 1 },
    ],
  ])('rejects a malformed request with %s', (_name, value) => {
    expect(isWorkerRequest(value)).toBe(false)
  })

  it('limits encoded imports by name and byte length without allocating', () => {
    expect(validateEncodedImport('n', 1)).toBeNull()
    expect(validateEncodedImport('x'.repeat(256), MAX_ENCODED_BYTES)).toBeNull()
    expect(validateEncodedImport('', 1)?.code).toBe('invalid_argument')
    expect(validateEncodedImport('x'.repeat(257), 1)?.code).toBe('invalid_argument')
    expect(validateEncodedImport('n', 0)?.code).toBe('invalid_argument')
    expect(validateEncodedImport('n', -1)?.code).toBe('invalid_argument')
    expect(validateEncodedImport('n', 1.5)?.code).toBe('invalid_argument')
    expect(validateEncodedImport('n', MAX_ENCODED_BYTES + 1)?.code).toBe('resource_limit_exceeded')
  })

  it('bounds original preview replies with the preview frame limits', () => {
    const reply = { type: 'previewed', requestId: 1, operationId: 1 }
    expect(
      isWorkerReply({
        ...reply,
        width: 1600,
        height: 900,
        pixels: new ArrayBuffer(1600 * 900 * 4),
      }),
    ).toBe(true)
    expect(
      isWorkerReply({ ...reply, width: 1601, height: 1, pixels: new ArrayBuffer(1601 * 4) }),
    ).toBe(false)
  })
})

describe('render mode and ramp contract', () => {
  it('keeps the classic binary default and forces average sampling for ascii and block', () => {
    expect(samplingForMode('binary')).toBe('classic')
    expect(samplingForMode('ascii')).toBe('average')
    expect(samplingForMode('block')).toBe('average')
  })

  it('accepts printable ASCII ramps from one to sixty-four characters', () => {
    expect(validateRamp(DEFAULT_RAMP)).toBeNull()
    expect(validateRamp('#')).toBeNull()
    expect(validateRamp(' ')).toBeNull()
    expect(validateRamp('!'.repeat(1).concat('~'.repeat(MAX_RAMP_LENGTH - 1)))).toBeNull()
  })

  it('names the failure for empty, non-printable, non-ASCII and oversized ramps', () => {
    expect(validateRamp('')).toBe(RAMP_EMPTY_MESSAGE)
    expect(validateRamp('\t')).toBe(RAMP_NOT_PRINTABLE_MESSAGE)
    expect(validateRamp('\u007f')).toBe(RAMP_NOT_PRINTABLE_MESSAGE)
    expect(validateRamp('caf\u00e9')).toBe(RAMP_NOT_PRINTABLE_MESSAGE)
    expect(validateRamp('\u{1f600}')).toBe(RAMP_NOT_PRINTABLE_MESSAGE)
    expect(validateRamp('a'.repeat(MAX_RAMP_LENGTH + 1))).toBe(RAMP_TOO_LONG_MESSAGE)
    expect(RAMP_TOO_LONG_MESSAGE).toBe('ramp must be at most 64 characters')
  })

  it('rejects worker requests whose mode or ramp has the wrong type or name', () => {
    const base = { type: 'preview', requestId: 1, operationId: 1, sourceId: 1 }
    const valid = { ...parameters, sampling: 'average', mode: 'ascii', ramp: ' .#' }
    expect(isWorkerRequest({ ...base, parameters: valid })).toBe(true)
    expect(isWorkerRequest({ ...base, parameters: { ...valid, mode: 'dots' } })).toBe(false)
    expect(isWorkerRequest({ ...base, parameters: { ...valid, mode: undefined } })).toBe(false)
    expect(isWorkerRequest({ ...base, parameters: { ...valid, ramp: 7 } })).toBe(false)
    expect(isWorkerRequest({ ...base, parameters: { ...valid, ramp: undefined } })).toBe(false)
  })
})
