import { describe, expect, it } from 'vitest'
import { footerStatusLabel, nextFooterAnnouncement } from './footerAnnouncement'
import type { FooterAnnouncementState } from './footerAnnouncement'

function at(status: string, exportStatus: FooterAnnouncementState['exportStatus']) {
  return { status, exportStatus, label: footerStatusLabel(status) }
}

describe('nextFooterAnnouncement', () => {
  it('keeps the label through export start, ready, failure and user cancel', () => {
    const ready = at('ready', 'idle')
    const exporting = nextFooterAnnouncement(ready, 'exporting', 'running')
    expect(exporting.label).toBe('Preview ready')
    expect(nextFooterAnnouncement(exporting, 'ready', 'ready').label).toBe('Preview ready')
    expect(nextFooterAnnouncement(exporting, 'error', 'failed').label).toBe('Preview ready')
    expect(nextFooterAnnouncement(exporting, 'cancelled', 'idle').label).toBe('Preview ready')
  })

  it('keeps a previous non-export label when an export runs over it', () => {
    const cancelledImport = at('cancelled', 'idle')
    const exporting = nextFooterAnnouncement(cancelledImport, 'exporting', 'running')
    expect(nextFooterAnnouncement(exporting, 'ready', 'ready').label).toBe('Operation cancelled')
  })

  it('speaks new work that supersedes an export', () => {
    const exporting = nextFooterAnnouncement(at('ready', 'idle'), 'exporting', 'running')
    expect(nextFooterAnnouncement(exporting, 'importing', 'idle').label).toBe('Importing image')
    expect(nextFooterAnnouncement(exporting, 'ready', 'idle').label).toBe('Preview ready')
  })

  it('speaks non-export cancellations', () => {
    const importing = at('importing', 'idle')
    expect(nextFooterAnnouncement(importing, 'cancelled', 'idle').label).toBe('Operation cancelled')
  })
})
