import type { BlobExportArtifact, ExportArtifact, UrlApi } from '../processing/types'

export interface CountingUrlApi extends UrlApi {
  readonly created: number
  readonly revoked: string[]
  readonly live: Set<string>
}

export function countingUrlApi(): CountingUrlApi {
  const state = { created: 0 }
  const revoked: string[] = []
  const live = new Set<string>()
  return {
    get created() {
      return state.created
    },
    revoked,
    live,
    createObjectURL() {
      state.created += 1
      const url = `blob:export/${state.created}`
      live.add(url)
      return url
    },
    revokeObjectURL(url) {
      revoked.push(url)
      live.delete(url)
    },
  }
}

export function artifactOf(size: number): BlobExportArtifact {
  return {
    kind: 'blob',
    format: 'png',
    blob: new Blob([new Uint8Array(size)], { type: 'image/png' }),
    byteLength: size,
    dispose: () => undefined,
  }
}

/** A text export artifact of `size` bytes with the UTF-8 text Blob type. */
export function textArtifactOf(size: number): BlobExportArtifact {
  return {
    kind: 'blob',
    format: 'text',
    blob: new Blob([new Uint8Array(size)], { type: 'text/plain;charset=utf-8' }),
    byteLength: size,
    dispose: () => undefined,
  }
}

/** Returns the blob of a browser export, failing the test if the artifact is a native save. */
export function blobOf(artifact: ExportArtifact): Blob {
  if (artifact.kind !== 'blob') throw new Error('Expected a blob export.')
  return artifact.blob
}
