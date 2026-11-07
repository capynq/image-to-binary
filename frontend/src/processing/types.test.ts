import { describe, expect, it } from 'vitest'
import { DEFAULT_RAMP } from './protocol'
import { DEFAULT_SETTINGS, validateSettings } from './types'
import type { StudioSettings } from './types'

const base: StudioSettings = {
  cellSize: 12,
  fontSize: 16,
  threshold: 128,
  mode: 'binary',
  ramp: DEFAULT_RAMP,
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
})
