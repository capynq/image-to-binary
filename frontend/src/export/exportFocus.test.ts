import { describe, expect, it } from 'vitest'
import { exportFocusTarget, shouldMoveExportFocus } from './exportFocus'

describe('export focus target', () => {
  it('moves focus to Cancel when an export starts, because Export was removed', () => {
    expect(exportFocusTarget('running', true)).toBe('cancel')
    expect(exportFocusTarget('running', false)).toBe('cancel')
  })

  it('moves focus to Download when the encoded PNG is ready', () => {
    expect(exportFocusTarget('ready', true)).toBe('download')
  })

  it('moves focus to Retry when an export fails', () => {
    expect(exportFocusTarget('failed', true)).toBe('retry')
  })

  it('leaves focus unchanged while the download link is still being prepared', () => {
    expect(exportFocusTarget('preparing', true)).toBeNull()
  })

  it('returns to Export when it is available and to the heading when it is not', () => {
    expect(exportFocusTarget('idle', true)).toBe('export')
    expect(exportFocusTarget('idle', false)).toBe('heading')
  })
})

describe('export focus ownership', () => {
  it('moves focus when it was lost to the body', () => {
    expect(shouldMoveExportFocus(true, false)).toBe(true)
  })

  it('moves focus when it sat inside the export panel', () => {
    expect(shouldMoveExportFocus(false, true)).toBe(true)
  })

  it('never takes focus away from a control elsewhere in the window', () => {
    expect(shouldMoveExportFocus(false, false)).toBe(false)
  })
})
