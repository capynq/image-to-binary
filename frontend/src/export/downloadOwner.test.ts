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

function textArtifact(size: number): BlobExportArtifact {
  return {
    kind: 'blob',
    format: 'text',
    blob: new Blob([new Uint8Array(size)], { type: 'text/plain;charset=utf-8' }),
    byteLength: size,
    dispose: () => undefined,
  }
}

function artifact(size: number): BlobExportArtifact {
  return {
    kind: 'blob',
    format: 'png',
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

describe('download owner for text exports', () => {
  it('owns one object URL for a .txt text artifact and reports its UTF-8 Blob type', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    const text = textArtifact(12)
    const link = owner.show(text, 'grid-binary.txt')
    expect(link?.filename).toBe('grid-binary.txt')
    expect(link?.artifact.blob.type).toBe('text/plain;charset=utf-8')
    expect(urls.created).toEqual(['blob:test/1:12'])
  })

  it('is idempotent for the same text artifact and name, and revokes the URL once on dispose', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    const text = textArtifact(4)
    const first = owner.show(text, 'grid-binary.txt')
    expect(owner.show(text, 'grid-binary.txt')).toBe(first)
    owner.dispose()
    owner.dispose()
    expect(urls.revoked).toEqual(['blob:test/1'])
  })

  it('replaces a PNG link with a text link, revoking the PNG URL exactly once', () => {
    const urls = countingUrlApi()
    const owner = createDownloadOwner(urls)
    owner.show(artifact(3), 'grid-binary.png')
    const text = owner.show(textArtifact(5), 'grid-binary.txt')
    expect(text?.url).toBe('blob:test/2')
    expect(urls.revoked).toEqual(['blob:test/1'])
    owner.show(null, '')
    expect(urls.revoked).toEqual(['blob:test/1', 'blob:test/2'])
  })
})
