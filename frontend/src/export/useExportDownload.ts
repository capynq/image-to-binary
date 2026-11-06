import { useEffect, useState } from 'react'
import type { ExportArtifact, UrlApi } from '../processing/types'
import { createDownloadOwner } from './downloadOwner'
import type { DownloadLink } from './downloadOwner'

/**
 * Binds the component to one download owner. The link is returned only while it belongs to the
 * current artifact, so a replaced or cleared export never exposes a revoked URL during the render
 * that precedes the owner's effect.
 */
export function useExportDownload(
  artifact: ExportArtifact | null,
  filename: string,
  urlApi: UrlApi,
): DownloadLink | null {
  const [owner] = useState(() => createDownloadOwner(urlApi))
  const [shown, setShown] = useState<DownloadLink | null>(null)

  useEffect(() => {
    setShown(owner.show(artifact, filename))
  }, [owner, artifact, filename])

  useEffect(() => () => owner.dispose(), [owner])

  return shown !== null && shown.artifact === artifact ? shown : null
}
