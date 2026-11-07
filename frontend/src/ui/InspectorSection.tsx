import { useId } from 'react'
import type { ReactNode, Ref } from 'react'

export interface InspectorSectionProps {
  readonly title: string
  readonly className?: string
  readonly children: ReactNode
  readonly sectionRef?: Ref<HTMLElement>
  readonly headingRef?: Ref<HTMLHeadingElement>
  /** Lets the heading take programmatic focus as a fallback target. */
  readonly focusableHeading?: boolean
}

/** An inspector section: a labelled `section` with its heading, then its content. */
export function InspectorSection({
  title,
  className,
  children,
  sectionRef,
  headingRef,
  focusableHeading = false,
}: InspectorSectionProps) {
  const headingId = useId()
  return (
    <section
      ref={sectionRef}
      className={`control-section ${className ?? ''}`.trim()}
      aria-labelledby={headingId}
    >
      <div className="section-heading">
        <h2 id={headingId} ref={headingRef} tabIndex={focusableHeading ? -1 : undefined}>
          {title}
        </h2>
      </div>
      {children}
    </section>
  )
}
