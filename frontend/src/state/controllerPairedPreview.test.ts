import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import { until } from '../processing/testKit'
import { DEFAULT_SETTINGS } from '../processing/types'
import type { PreviewKind, ProcessingAdapter, StudioSettings } from '../processing/types'
import { createStudioController } from './controller'
import { DEFAULT_RAMP } from '../processing/protocol'

function gateway(): { readonly promise: Promise<void>; release(): void } {
  let release: () => void = () => undefined
  const promise = new Promise<void>((resolve) => {
    release = resolve
  })
  return { promise, release: () => release() }
}

/** Counts every artifact and source disposal call so duplicate releases are visible. */
function countingAdapter(): {
  readonly adapter: ProcessingAdapter
  readonly kinds: Array<PreviewKind | undefined>
  readonly revoked: string[]
  readonly sourcesDisposed: string[]
  readonly control: {
    failNextImport: boolean
    failNextPreview: boolean
    importGate: Promise<void> | null
    previewGate: Promise<void> | null
  }
} {
  const base = createDemoProcessingAdapter()
  const kinds: Array<PreviewKind | undefined> = []
  const revoked: string[] = []
  const sourcesDisposed: string[] = []
  const control: {
    failNextImport: boolean
    failNextPreview: boolean
    importGate: Promise<void> | null
    previewGate: Promise<void> | null
  } = { failNextImport: false, failNextPreview: false, importGate: null, previewGate: null }
  let imports = 0
  let artifacts = 0
  const adapter: ProcessingAdapter = {
    createSession: () => base.createSession(),
    import: async (session, input, operation) => {
      const id = ++imports
      const gate = control.importGate
      control.importGate = null
      await gate
      if (control.failNextImport) {
        control.failNextImport = false
        throw new Error('Import rejected.')
      }
      const result = await base.import(session, input, operation)
      return {
        ...result,
        metadata: { name: `Image ${id}`, width: 4, height: 4, source: 'file', inputBytes: 3 },
        dispose: () => {
          sourcesDisposed.push(`import-${id}`)
          result.dispose()
        },
      }
    },
    preview: async (_session, _source, _settings, _operation, kind) => {
      kinds.push(kind)
      const gate = control.previewGate
      control.previewGate = null
      await gate
      if (control.failNextPreview) {
        control.failNextPreview = false
        throw new Error('Preview rejected.')
      }
      const url = `blob:${kind ?? 'artwork'}-${++artifacts}`
      return {
        url,
        width: 4,
        height: 2,
        dispose: () => {
          revoked.push(url)
        },
      }
    },
    export: (session, source, format, settings, operation) =>
      base.export(session, source, format, settings, operation),
    cancel: (session, operationId) => base.cancel(session, operationId),
    dispose: (session) => base.dispose(session),
  }
  return { adapter, kinds, revoked, sourcesDisposed, control }
}

describe('controller paired previews', () => {
  it('commits original and artwork independently without touching settings', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    expect(controller.getSnapshot().original).toBeNull()
    expect(controller.getSnapshot().preview).toBeNull()
    await controller.loadDemo()
    expect(controller.getSnapshot().metadata?.source).toBe('file')
    const settings = controller.getSnapshot().settings
    await controller.preview('original')
    const original = controller.getSnapshot().original
    expect(original).toEqual({ url: 'blob:original-1', width: 4, height: 2 })
    expect(controller.getSnapshot().preview).toBeNull()
    expect(controller.getSnapshot().settings).toBe(settings)
    await controller.preview()
    expect(controller.getSnapshot().preview).toEqual({
      url: 'blob:artwork-2',
      width: 4,
      height: 2,
    })
    expect(controller.getSnapshot().original).toBe(original)
    expect(h.kinds).toEqual(['original', 'artwork'])
    const snapshot = controller.getSnapshot()
    expect(Object.isFrozen(snapshot)).toBe(true)
    expect(Object.isFrozen(snapshot.original)).toBe(true)
    expect(JSON.parse(JSON.stringify(snapshot))).toEqual(snapshot)
  })

  it('keeps stale artwork on settings change and releases it once when replaced', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview('original')
    await controller.preview()
    const artwork = controller.getSnapshot().preview
    controller.updateSettings({
      cellSize: 14,
      fontSize: 16,
      threshold: 128,
      mode: 'binary' as const,
      ramp: DEFAULT_RAMP,
      foreground: '#00ff00',
      background: '#000000',
      colorMode: 'monochrome',
      contrast: 0,
      invert: false,
    })
    controller.updateSettings({
      cellSize: 15,
      fontSize: 16,
      threshold: 128,
      mode: 'binary' as const,
      ramp: DEFAULT_RAMP,
      foreground: '#00ff00',
      background: '#000000',
      colorMode: 'monochrome',
      contrast: 0,
      invert: false,
    })
    expect(h.revoked).toEqual([])
    expect(controller.getSnapshot().preview).toBe(artwork)
    expect(controller.getSnapshot().previewRevision).toBe(0)
    expect(controller.getSnapshot().settingsRevision).toBe(2)
    expect(controller.getSnapshot().original?.url).toBe('blob:original-1')
    expect(controller.getSnapshot().settings.cellSize).toBe(15)
    expect(controller.getSnapshot().status).toBe('ready')
    await controller.preview()
    expect(h.revoked).toEqual(['blob:artwork-2'])
    expect(controller.getSnapshot().preview?.url).toBe('blob:artwork-3')
    expect(controller.getSnapshot().previewRevision).toBe(2)
  })

  it('retains the pair and source when a replacement import fails', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview('original')
    await controller.preview()
    const before = controller.getSnapshot()
    h.control.failNextImport = true
    await expect(controller.loadDemo()).rejects.toThrow('Import rejected.')
    const after = controller.getSnapshot()
    expect(after.metadata).toBe(before.metadata)
    expect(after.preview).toBe(before.preview)
    expect(after.original).toBe(before.original)
    expect(after.error).toBe('Import rejected. Previous image remains available.')
    expect(h.sourcesDisposed).toEqual([])
    expect(h.revoked).toEqual([])
    expect(await (await controller.exportDemo('text')).text()).toBe('1010\n0101\n1010\n0101\n')
  })

  it.each(['cancel', 'supersede'] as const)(
    'retains the pair and source when a replacement import is %s',
    async (action) => {
      const h = countingAdapter()
      const controller = createStudioController({ adapter: h.adapter })
      await controller.loadDemo()
      await controller.preview('original')
      await controller.preview()
      const before = controller.getSnapshot()
      const gate = gateway()
      h.control.importGate = gate.promise
      const pending = controller.loadDemo()
      if (action === 'cancel') {
        controller.cancel()
      } else {
        await controller.exportDemo('text')
      }
      gate.release()
      await pending
      const after = controller.getSnapshot()
      expect(after.metadata).toBe(before.metadata)
      expect(after.preview).toBe(before.preview)
      expect(after.original).toBe(before.original)
      expect(h.sourcesDisposed).toEqual(['import-2'])
      expect(h.revoked).toEqual([])
    },
  )

  it('releases the pair and previous source exactly once after an accepted replacement', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview('original')
    await controller.preview()
    const gate = gateway()
    h.control.importGate = gate.promise
    const pending = controller.loadDemo()
    expect(h.revoked).toEqual([])
    expect(h.sourcesDisposed).toEqual([])
    gate.release()
    await pending
    expect([...h.revoked].sort()).toEqual(['blob:artwork-2', 'blob:original-1'])
    expect(h.sourcesDisposed).toEqual(['import-1'])
    expect(controller.getSnapshot().preview).toBeNull()
    expect(controller.getSnapshot().original).toBeNull()
    expect(controller.getSnapshot().metadata?.name).toBe('Image 2')
  })

  it.each(['import', 'reset', 'dispose'] as const)(
    'disposes a late original once after %s',
    async (action) => {
      const h = countingAdapter()
      const controller = createStudioController({ adapter: h.adapter })
      await controller.loadDemo()
      const gate = gateway()
      h.control.previewGate = gate.promise
      const pending = controller.preview('original')
      await until(() => h.kinds.length === 1, 'original request')
      let next: Promise<void> = Promise.resolve()
      if (action === 'import') {
        next = controller.loadDemo()
      } else if (action === 'reset') {
        controller.reset()
      } else {
        controller.dispose()
      }
      gate.release()
      await pending
      await next
      expect(h.revoked).toEqual(['blob:original-1'])
      expect(controller.getSnapshot().original).toBeNull()
    },
  )

  it('keeps an accepted original when a pending artwork is cancelled', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview('original')
    const gate = gateway()
    h.control.previewGate = gate.promise
    const pending = controller.preview()
    await until(() => h.kinds.length === 2, 'artwork request')
    controller.cancel()
    gate.release()
    await pending
    expect(controller.getSnapshot().status).toBe('cancelled')
    expect(controller.getSnapshot().original?.url).toBe('blob:original-1')
    expect(controller.getSnapshot().preview).toBeNull()
    expect(h.revoked).toEqual(['blob:artwork-2'])
  })

  it('releases both artifacts and the source idempotently on reset and dispose', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview('original')
    await controller.preview()
    controller.reset()
    controller.reset()
    expect([...h.revoked].sort()).toEqual(['blob:artwork-2', 'blob:original-1'])
    expect(h.sourcesDisposed).toEqual(['import-1'])
    expect(controller.getSnapshot().original).toBeNull()
    expect(controller.getSnapshot().preview).toBeNull()
    await controller.loadDemo()
    await controller.preview('original')
    await controller.preview()
    controller.dispose()
    controller.dispose()
    expect(h.revoked).toHaveLength(4)
    expect(h.sourcesDisposed).toEqual(['import-1', 'import-2'])
    expect(controller.getSnapshot().status).toBe('disposed')
    expect(controller.getSnapshot().original).toBeNull()
    expect(controller.getSnapshot().preview).toBeNull()
  })

  it('rejects an invalid preview kind before mutating state', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview('original')
    const before = controller.getSnapshot()
    let notifications = 0
    controller.subscribe(() => {
      notifications += 1
    })
    await expect(controller.preview('bogus' as PreviewKind)).rejects.toThrow('Unknown preview kind')
    expect(controller.getSnapshot()).toBe(before)
    expect(notifications).toBe(0)
    expect(h.kinds).toEqual(['original'])
    expect(h.revoked).toEqual([])
  })

  it('keeps the accepted source and artwork when the adapter lacks original previews', async () => {
    const revoked: string[] = []
    let created = 0
    const adapter = createDemoProcessingAdapter({
      urlApi: {
        createObjectURL: () => `blob:demo-${++created}`,
        revokeObjectURL: (url) => {
          revoked.push(url)
        },
      },
    })
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    await controller.preview()
    const artwork = controller.getSnapshot().preview
    await expect(controller.preview('original')).rejects.toThrow(
      'Original preview is not available',
    )
    const snapshot = controller.getSnapshot()
    expect(snapshot.status).toBe('error')
    expect(snapshot.error).toBe(
      'Original preview is not available in the demo-only adapter. Previous image remains available.',
    )
    expect(snapshot.original).toBeNull()
    expect(snapshot.preview).toBe(artwork)
    expect(snapshot.metadata?.name).toBe('Demo fixture')
    expect(revoked).toEqual([])
    expect(await (await controller.exportDemo('text')).text()).toBe('1010\n0101\n1010\n0101\n')
  })
})

function withCellSize(cellSize: number): StudioSettings {
  return { ...DEFAULT_SETTINGS, cellSize }
}

describe('stale artwork and quiet previews', () => {
  it('advances the settings revision only for accepted changes', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    expect(controller.getSnapshot().settingsRevision).toBe(0)
    expect(controller.getSnapshot().previewRevision).toBeNull()
    expect(controller.getSnapshot().artworkUpdating).toBe(false)
    await controller.loadDemo()
    await controller.preview()
    expect(controller.getSnapshot().previewRevision).toBe(0)
    controller.updateSettings(withCellSize(15))
    expect(controller.getSnapshot().settingsRevision).toBe(1)
    expect(controller.getSnapshot().previewRevision).toBe(0)
    controller.updateSettings(withCellSize(15))
    expect(controller.getSnapshot().settingsRevision).toBe(1)
    expect(() => controller.updateSettings(withCellSize(4))).toThrow('Cell size')
    expect(controller.getSnapshot().settingsRevision).toBe(1)
    await controller.preview()
    expect(controller.getSnapshot().previewRevision).toBe(1)
  })

  it('keeps the artwork and its revision through a settings change and resets them on import', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    controller.updateSettings(withCellSize(15))
    expect(controller.getSnapshot().preview?.url).toBe('blob:artwork-1')
    expect(controller.getSnapshot().previewRevision).toBe(0)
    await controller.loadDemo()
    expect(controller.getSnapshot().preview).toBeNull()
    expect(controller.getSnapshot().previewRevision).toBeNull()
    expect(controller.getSnapshot().settingsRevision).toBe(1)
    expect(h.revoked).toEqual(['blob:artwork-1'])
  })

  it('supersedes an in-flight render on settings change without touching the displayed artwork', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    const artwork = controller.getSnapshot().preview
    const gate = gateway()
    h.control.previewGate = gate.promise
    const pending = controller.preview()
    await until(() => h.kinds.length === 2, 'artwork request')
    controller.updateSettings(withCellSize(15))
    expect(controller.getSnapshot().preview).toBe(artwork)
    gate.release()
    await pending
    expect(controller.getSnapshot().preview).toBe(artwork)
    expect(controller.getSnapshot().previewRevision).toBe(0)
    expect(h.revoked).toEqual(['blob:artwork-2'])
  })

  it('disposes the retained artwork exactly once on dispose after a settings change', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    controller.updateSettings(withCellSize(15))
    controller.dispose()
    controller.dispose()
    expect(h.revoked).toEqual(['blob:artwork-1'])
    expect(controller.getSnapshot().preview).toBeNull()
    expect(controller.getSnapshot().previewRevision).toBeNull()
  })

  it('keeps ready status and toggles artworkUpdating around a quiet render', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    const statuses: string[] = []
    const updating: boolean[] = []
    controller.subscribe(() => {
      statuses.push(controller.getSnapshot().status)
      updating.push(controller.getSnapshot().artworkUpdating)
    })
    const gate = gateway()
    h.control.previewGate = gate.promise
    const pending = controller.preview('artwork', { quiet: true })
    await until(() => h.kinds.length === 2, 'quiet request')
    expect(controller.getSnapshot().status).toBe('ready')
    expect(controller.getSnapshot().progress).toBe(1)
    expect(controller.getSnapshot().artworkUpdating).toBe(true)
    expect(controller.getSnapshot().preview?.url).toBe('blob:artwork-1')
    gate.release()
    await pending
    expect(controller.getSnapshot().artworkUpdating).toBe(false)
    expect(controller.getSnapshot().status).toBe('ready')
    expect(controller.getSnapshot().preview?.url).toBe('blob:artwork-2')
    expect(controller.getSnapshot().previewRevision).toBe(0)
    expect(h.revoked).toEqual(['blob:artwork-1'])
    expect(statuses).not.toContain('processing')
    expect(updating).toEqual([true, false])
  })

  it('a quiet render publishes fresh artwork that clears staleness', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    controller.updateSettings(withCellSize(15))
    expect(controller.getSnapshot().previewRevision).not.toBe(
      controller.getSnapshot().settingsRevision,
    )
    await controller.preview('artwork', { quiet: true })
    expect(controller.getSnapshot().previewRevision).toBe(controller.getSnapshot().settingsRevision)
    expect(controller.getSnapshot().artworkUpdating).toBe(false)
    expect(h.revoked).toEqual(['blob:artwork-1'])
  })

  it('keeps the old artwork and surfaces the error when a quiet render fails', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    const artwork = controller.getSnapshot().preview
    h.control.failNextPreview = true
    await expect(controller.preview('artwork', { quiet: true })).rejects.toThrow(
      'Preview rejected.',
    )
    const snapshot = controller.getSnapshot()
    expect(snapshot.artworkUpdating).toBe(false)
    expect(snapshot.status).toBe('error')
    expect(snapshot.error).toBe('Preview rejected. Previous image remains available.')
    expect(snapshot.preview).toBe(artwork)
    expect(snapshot.previewRevision).toBe(0)
    expect(h.revoked).toEqual([])
  })

  it('rejects a quiet preview of the original before any state changes', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    const before = controller.getSnapshot()
    await expect(controller.preview('original', { quiet: true })).rejects.toThrow(
      'Only the artwork preview can be quiet.',
    )
    expect(controller.getSnapshot()).toBe(before)
    expect(h.kinds).toEqual([])
  })

  it.each(['cancel', 'settings', 'preview', 'import', 'reset', 'dispose'] as const)(
    '%s clears artworkUpdating in the same commit that supersedes a quiet render',
    async (action) => {
      const h = countingAdapter()
      const controller = createStudioController({ adapter: h.adapter })
      await controller.loadDemo()
      await controller.preview()
      const gate = gateway()
      h.control.previewGate = gate.promise
      const pending = controller.preview('artwork', { quiet: true })
      await until(() => h.kinds.length === 2, 'quiet request')
      expect(controller.getSnapshot().artworkUpdating).toBe(true)
      const seen: boolean[] = []
      controller.subscribe(() => seen.push(controller.getSnapshot().artworkUpdating))
      let next: Promise<unknown> = Promise.resolve()
      if (action === 'cancel') controller.cancel()
      else if (action === 'settings') controller.updateSettings(withCellSize(15))
      else if (action === 'preview') next = controller.preview()
      else if (action === 'import') next = controller.loadDemo()
      else if (action === 'reset') controller.reset()
      else controller.dispose()
      expect(controller.getSnapshot().artworkUpdating).toBe(false)
      expect(seen[0]).toBe(false)
      gate.release()
      await pending
      await next
      expect(controller.getSnapshot().artworkUpdating).toBe(false)
      expect(new Set(h.revoked).size).toBe(h.revoked.length)
      const shown = controller.getSnapshot().preview?.url
      if (shown !== undefined) expect(h.revoked).not.toContain(shown)
      expect(h.revoked.length).toBeGreaterThan(0)
    },
  )

  it('discards and disposes a stale quiet result after a newer settings change', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    const gate = gateway()
    h.control.previewGate = gate.promise
    const pending = controller.preview('artwork', { quiet: true })
    await until(() => h.kinds.length === 2, 'quiet request')
    controller.updateSettings(withCellSize(15))
    gate.release()
    await pending
    const snapshot = controller.getSnapshot()
    expect(h.revoked).toEqual(['blob:artwork-2'])
    expect(snapshot.preview?.url).toBe('blob:artwork-1')
    expect(snapshot.previewRevision).toBe(0)
    expect(snapshot.settingsRevision).toBe(1)
    expect(snapshot.artworkUpdating).toBe(false)
    expect(snapshot.status).toBe('ready')
  })

  it('exports with the new settings while stale artwork stays displayed', async () => {
    const h = countingAdapter()
    const controller = createStudioController({ adapter: h.adapter })
    await controller.loadDemo()
    await controller.preview()
    const before = await (await controller.exportDemo('text')).text()
    const artwork = controller.getSnapshot().preview
    const changed: StudioSettings = { ...DEFAULT_SETTINGS, threshold: 0 }
    controller.updateSettings(changed)
    expect(controller.getSnapshot().preview).toBe(artwork)
    expect(controller.getSnapshot().previewRevision).not.toBe(
      controller.getSnapshot().settingsRevision,
    )
    const after = await (await controller.exportDemo('text')).text()
    expect(after).not.toBe(before)
    const reference = createStudioController({
      adapter: countingAdapter().adapter,
      settings: changed,
    })
    await reference.loadDemo()
    expect(after).toBe(await (await reference.exportDemo('text')).text())
  })
})
