import type { ReactNode } from 'react'

export interface MetadataItem {
  /** Term shown before the value; also the React key, so it must be unique within a list. */
  readonly label: string
  /** Full value, rendered without truncation. */
  readonly value: ReactNode
}

/**
 * A definition list of metadata. Values are shown in full: names and sizes wrap rather than being
 * truncated, so no information is hidden by overflow.
 */
export function MetadataList({ items }: { readonly items: readonly MetadataItem[] }) {
  return (
    <dl className="metadata-list">
      {items.map((item) => (
        <div key={item.label}>
          <dt>{item.label}</dt>
          <dd>{item.value}</dd>
        </div>
      ))}
    </dl>
  )
}
