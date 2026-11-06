import type { ExportArtifact, UrlApi } from '../processing/types'

/** One object URL for one encoded export. `dispose` revokes the URL exactly once. */
export interface DownloadLink {
  readonly artifact: ExportArtifact
  readonly url: string
  readonly filename: string
  dispose(): void
}

/**
 * Sole owner of the download object URL. `show` is idempotent for the same artifact and
 * filename, creates one URL for a new artifact and revokes the previous one, and revokes
 * everything for `null`. `dispose` revokes the current URL; a later `show` may create another.
 * Revocation never happens at click time, so the browser's download is not aborted.
 */
export interface DownloadOwner {
  show(artifact: ExportArtifact | null, filename: string): DownloadLink | null
  dispose(): void
}

export function createDownloadLink(
  artifact: ExportArtifact,
  filename: string,
  urlApi: UrlApi,
): DownloadLink {
  const url = urlApi.createObjectURL(artifact.blob)
  let revoked = false
  return {
    artifact,
    url,
    filename,
    dispose(): void {
      if (revoked) return
      revoked = true
      urlApi.revokeObjectURL(url)
    },
  }
}

export function createDownloadOwner(urlApi: UrlApi): DownloadOwner {
  let current: DownloadLink | null = null
  return {
    show(artifact, filename) {
      if (
        current !== null &&
        artifact !== null &&
        current.artifact === artifact &&
        current.filename === filename
      ) {
        return current
      }
      current?.dispose()
      current = artifact === null ? null : createDownloadLink(artifact, filename, urlApi)
      return current
    },
    dispose() {
      current?.dispose()
      current = null
    },
  }
}
