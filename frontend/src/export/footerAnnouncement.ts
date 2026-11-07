import { useState } from 'react'
import type { ExportStatus } from '../processing/types'

export function footerStatusLabel(status: string): string {
  switch (status) {
    case 'idle':
      return 'Ready for an image'
    case 'importing':
      return 'Importing image'
    case 'processing':
      return 'Rendering preview'
    case 'exporting':
      return 'Preparing export'
    case 'cancelled':
      return 'Operation cancelled'
    case 'error':
      return 'Action needs attention'
    case 'disposed':
      return 'Studio closed'
    default:
      return 'Preview ready'
  }
}

export interface FooterAnnouncementState {
  readonly status: string
  readonly exportStatus: ExportStatus
  readonly label: string
}

/**
 * An export start, finish, failure or user cancel is spoken by the export panel, and an import or
 * preview error is spoken by its role=alert panel, so both keep the previous label. Import, preview
 * and supersession by a setting or source change are new work and speak their own label.
 */
export function nextFooterAnnouncement(
  previous: FooterAnnouncementState,
  status: string,
  exportStatus: ExportStatus,
): FooterAnnouncementState {
  const exportTransition =
    status === 'exporting' ||
    (previous.status === 'exporting' && (exportStatus !== 'idle' || status === 'cancelled'))
  const spokenByAlert = status === 'error'
  return {
    status,
    exportStatus,
    label: exportTransition || spokenByAlert ? previous.label : footerStatusLabel(status),
  }
}

/**
 * Returns the label of the footer's visually hidden live region. The region changes only for
 * non-export transitions, so the export lifecycle is never announced twice.
 */
export function useFooterAnnouncement(status: string, exportStatus: ExportStatus): string {
  const [state, setState] = useState<FooterAnnouncementState>(() => ({
    status,
    exportStatus,
    label: footerStatusLabel(status),
  }))
  if (state.status === status && state.exportStatus === exportStatus) return state.label
  const next = nextFooterAnnouncement(state, status, exportStatus)
  setState(next)
  return next.label
}
