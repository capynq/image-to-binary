import { describe, expect, it } from 'vitest'
import type { BlobExportArtifact, UrlApi } from '../processing/types'
import { createDownloadOwner } from './downloadOwner'

function countingUrlApi(): UrlApi & { created: string[]; revoked: string[] } {
  const created: string[] = []
  const revoked: string[] = []
  return {
    created,
    revoked,
    createObjectURL(object) {
      const url = `blob:test/${created.length + 1}`
      created.push(`${url}:${object.size}`)
      return url
    },
    revokeObjectURL(url) {
      revoked.push(url)
    },
  }
}

function artifact(size: number): BlobExportArtifact {
  return {
    kind: 'blob',
    blob: new Blob([new Uint8Array(size)], { type: 'image/png' }),
    byteLength: size,
    dispose: () => undefined,
  }
}

describe('download owner', () => {
  it('creates one URL per artifact and reuses it for repeated renders', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    const first = artifact(3)
    const link = owner.show(first, 'photo-binary.png')
    expect(owner.show(first, 'photo-binary.png')).toBe(link)
    expect(owner.show(first, 'photo-binary.png')).toBe(link)
    expect(urls.created).toHaveLength(1)
    expect(urls.revoked).toHaveLength(0)
    expect(link?.url).toBe('blob:test/1')
    expect(link?.filename).toBe('photo-binary.png')
  })

  it('revokes the previous URL exactly once when the artifact is replaced', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    owner.show(artifact(3), 'photo-binary.png')
    const replacement = owner.show(artifact(4), 'photo-binary.png')
    expect(urls.created).toHaveLength(2)
    expect(urls.revoked).toEqual(['blob:test/1'])
    expect(replacement?.url).toBe('blob:test/2')
  })

  it('revokes when the filename changes for the same artifact', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    const shared = artifact(3)
    owner.show(shared, 'a-binary.png')
    owner.show(shared, 'b-binary.png')
    expect(urls.created).toHaveLength(2)
    expect(urls.revoked).toEqual(['blob:test/1'])
  })

  it('revokes on clearing (reset or settings invalidation) and returns no link', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    owner.show(artifact(3), 'photo-binary.png')
    expect(owner.show(null, 'photo-binary.png')).toBeNull()
    expect(owner.show(null, 'photo-binary.png')).toBeNull()
    expect(urls.created).toHaveLength(1)
    expect(urls.revoked).toEqual(['blob:test/1'])
  })

  it('revokes on disposal exactly once even when disposed repeatedly', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    owner.show(artifact(3), 'photo-binary.png')
    owner.dispose()
    owner.dispose()
    expect(urls.revoked).toEqual(['blob:test/1'])
  })

  it('creates a fresh URL after disposal when the same artifact is shown again', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    const shared = artifact(3)
    owner.show(shared, 'photo-binary.png')
    owner.dispose()
    owner.show(shared, 'photo-binary.png')
    expect(urls.created).toHaveLength(2)
    expect(urls.revoked).toEqual(['blob:test/1'])
    owner.dispose()
    expect(urls.revoked).toEqual(['blob:test/1', 'blob:test/2'])
  })

  it('balances creations and revocations over repeated export cycles', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    for (let cycle = 0; cycle < 20; cycle += 1) {
      owner.show(artifact(cycle + 1), 'photo-binary.png')
      owner.show(null, 'photo-binary.png')
    }
    owner.dispose()
    expect(urls.created).toHaveLength(20)
    expect(urls.revoked).toHaveLength(20)
    expect(new Set(urls.revoked).size).toBe(20)
  })
})
