import { describe, expect, it } from 'vitest'
import { announceExportPhase } from './exportAnnouncement'
import type { AnnouncedExport } from './exportAnnouncement'

const running: AnnouncedExport = { phase: 'running', text: 'Rendering PNG export.' }

describe('announceExportPhase', () => {
  it('speaks the start of a render and the ready size', () => {
    expect(announceExportPhase(null, 'running', null, null)).toBe('Rendering PNG export.')
    expect(announceExportPhase(running, 'ready', 2048, null)).toBe('PNG export ready, 2.0 KiB.')
  })

  it('keeps the running text through the download step', () => {
    expect(announceExportPhase(running, 'preparing', null, null)).toBe('Rendering PNG export.')
    expect(announceExportPhase(null, 'preparing', null, null)).toBe('Preparing PNG download.')
  })

  it('announces a running-to-idle transition as a cancellation', () => {
    expect(announceExportPhase(running, 'idle', null, null)).toBe('PNG export cancelled.')
  })

  it('stays silent when idle follows a ready or failed export', () => {
    const ready: AnnouncedExport = { phase: 'ready', text: 'PNG export ready, 2.0 KiB.' }
    const failed: AnnouncedExport = { phase: 'failed', text: 'PNG export failed.' }
    expect(announceExportPhase(ready, 'idle', null, null)).toBe('')
    expect(announceExportPhase(failed, 'idle', null, null)).toBe('')
    expect(announceExportPhase(null, 'idle', null, null)).toBe('')
  })

  it('announces the failure together with the full error message', () => {
    const message = 'The PNG encoder stopped after 3 of 9 chunks. Reduce the cell size.'
    expect(announceExportPhase(running, 'failed', null, message)).toBe(
      `PNG export failed. ${message}`,
    )
    expect(announceExportPhase(running, 'failed', null, null)).toBe('PNG export failed.')
  })
})
