import type { ReactNode } from 'react'

export interface StatusRegionProps {
  readonly children: ReactNode
  readonly className?: string
}

/**
 * A polite status live region. Keep one per lifecycle: the owner decides what text it holds, and
 * a region that re-renders with the same text does not announce again.
 */
export function StatusRegion({ children, className }: StatusRegionProps) {
  return (
    <p className={className} role="status" aria-live="polite">
      {children}
    </p>
  )
}
