import { formatBytes } from './useImportFlow'
import type { StudioSnapshot } from './processing/types'
import { InspectorSection, MetadataList } from './ui'
import type { MetadataItem } from './ui'

export type MetadataSource = Pick<StudioSnapshot, 'metadata' | 'original' | 'preview'>

/** Inspector metadata: every name, size and dimension in full, or a prompt when nothing is open. */
export function MetadataSection({ metadata, original, preview }: MetadataSource) {
  if (metadata === null) {
    return (
      <InspectorSection title="Metadata">
        <p className="section-help">Open an image to inspect its metadata.</p>
      </InspectorSection>
    )
  }
  const items: MetadataItem[] = [
    { label: 'Filename', value: metadata.name },
    { label: 'Dimensions', value: `${metadata.width} × ${metadata.height} px` },
    ...(metadata.inputBytes === undefined
      ? []
      : [{ label: 'File size', value: formatBytes(metadata.inputBytes) }]),
    {
      label: 'Source',
      value: metadata.source === 'demo' ? 'Built-in demo' : 'Local file',
    },
    ...(original === null
      ? []
      : [{ label: 'Original preview', value: `${original.width} × ${original.height} px` }]),
    ...(preview === null
      ? []
      : [{ label: 'Artwork preview', value: `${preview.width} × ${preview.height} px` }]),
  ]
  return (
    <InspectorSection title="Metadata">
      <MetadataList items={items} />
    </InspectorSection>
  )
}
