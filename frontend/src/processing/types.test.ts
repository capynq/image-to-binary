import { describe, expect, it } from 'vitest'
import { DEFAULT_RAMP, INVERT_BOOLEAN_MESSAGE } from './protocol'
import { DEFAULT_SETTINGS, validateSettings } from './types'
import type { StudioSettings } from './types'

const base: StudioSettings = {
  cellSize: 12,
  fontSize: 16,
  threshold: 128,
  mode: 'binary',
  ramp: DEFAULT_RAMP,
  foreground: '#00ff00',
  background: '#000000',
  colorMode: 'monochrome',
  contrast: 0,
  invert: false,
}

describe('studio settings', () => {
  it('defaults to the classic binary mode with the default ramp', () => {
    expect(DEFAULT_SETTINGS.mode).toBe('binary')
    expect(DEFAULT_SETTINGS.ramp).toBe(DEFAULT_RAMP)
    expect(() => validateSettings(DEFAULT_SETTINGS)).not.toThrow()
  })

  it('accepts ascii and block with a valid ramp', () => {
    expect(() => validateSettings({ ...base, mode: 'ascii', ramp: ' .#@' })).not.toThrow()
    expect(() => validateSettings({ ...base, mode: 'block' })).not.toThrow()
  })

  it('rejects an invalid ramp with its readable message even outside ascii mode', () => {
    expect(() => validateSettings({ ...base, mode: 'ascii', ramp: '' })).toThrow(
      'ramp must contain at least one character',
    )
    expect(() => validateSettings({ ...base, ramp: 'caf\u00e9' })).toThrow(
      'ramp must contain only printable ASCII characters',
    )
  })

  it('rejects an unknown render mode', () => {
    expect(() =>
      validateSettings({ ...base, mode: 'dots' as unknown as StudioSettings['mode'] }),
    ).toThrow('Render mode must be binary, ascii or block.')
  })

  it('keeps the defaults of the style fields unchanged in the default settings', () => {
    expect(DEFAULT_SETTINGS).toMatchObject({
      foreground: '#00ff00',
      background: '#000000',
      colorMode: 'monochrome',
      contrast: 0,
      invert: false,
    })
  })

  it('accepts the style fields at their bounds and in source colour', () => {
    expect(() => validateSettings({ ...base, contrast: -100, invert: true })).not.toThrow()
    expect(() => validateSettings({ ...base, contrast: 100, colorMode: 'source' })).not.toThrow()
    expect(() => validateSettings({ ...base, foreground: '#ABCDEF' })).not.toThrow()
  })

  it.each([
    [{ foreground: '#fff' }, 'foreground must be a #RRGGBB colour'],
    [{ background: 'black' }, 'background must be a #RRGGBB colour'],
    [
      { colorMode: 'color' as unknown as StudioSettings['colorMode'] },
      'colorMode must be "monochrome" or "source"',
    ],
    [{ contrast: 0.25 }, 'contrast must be an integer'],
    [{ contrast: 101 }, 'contrast must be between -100 and 100'],
    [{ contrast: -101 }, 'contrast must be between -100 and 100'],
  ] as const)('rejects %j with its readable message', (override, message) => {
    expect(() => validateSettings({ ...base, ...override })).toThrow(new RangeError(message))
  })

  it('rejects a non-boolean invert', () => {
    expect(() =>
      validateSettings({ ...base, invert: 'yes' as unknown as StudioSettings['invert'] }),
    ).toThrow(INVERT_BOOLEAN_MESSAGE)
  })

  it('keeps the cell, font and threshold messages while reading the shared bounds', () => {
    expect(() => validateSettings({ ...base, cellSize: 4 })).toThrow(
      'Cell size must be an integer from 5 through 30.',
    )
    expect(() => validateSettings({ ...base, fontSize: 31 })).toThrow(
      'Font size must be an integer from 8 through 30.',
    )
    expect(() => validateSettings({ ...base, threshold: 256 })).toThrow(
      'Threshold must be an integer from 0 through 255.',
    )
  })
})
