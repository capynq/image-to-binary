import type { ExportArtifact, UrlApi } from '../processing/types'

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

export function artifactOf(size: number): ExportArtifact {
  return {
    blob: new Blob([new Uint8Array(size)], { type: 'image/png' }),
    byteLength: size,
    dispose: () => undefined,
  }
}
