import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import { createStudioController } from './controller'
import { DEFAULT_RAMP } from '../processing/protocol'

describe('studio controller', () => {
  it('loads the demo, renders a preview, and exports the same fixture as text', async () => {
    const urls = createUrlApi()
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({ urlApi: urls }),
    })
    const before = controller.getSnapshot()

    await controller.loadDemo()
    expect(controller.getSnapshot()).not.toBe(before)
    expect(controller.getSnapshot().metadata).toEqual({
      name: 'Demo fixture',
      width: 4,
      height: 4,
      source: 'demo',
    })
    await controller.preview()

    const preview = controller.getSnapshot().preview
    expect(preview?.url).toBe('blob:demo-1')
    expect(preview?.width).toBe(48)

    const text = await controller.exportDemo('text')
    expect(await text.text()).toBe('1010\n0101\n1010\n0101')
    expect(controller.getSnapshot().status).toBe('ready')
  })

  it('rejects file input and unsupported PNG export without claiming conversion', async () => {
    const controller = createStudioController()
    await expect(controller.import({} as File)).rejects.toThrow('File conversion')
    expect(controller.getSnapshot().status).toBe('error')
    expect(controller.getSnapshot().error).toContain('File conversion')

    await controller.loadDemo()
    await expect(controller.exportDemo('png')).rejects.toThrow('PNG export')
    expect(controller.getSnapshot().status).toBe('error')
  })

  it('validates settings and reset revokes a preview while preserving cancellation safety', async () => {
    const urls = createUrlApi()
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({ urlApi: urls }),
    })
    await controller.loadDemo()
    await controller.preview()

    expect(() =>
      controller.updateSettings({
        cellSize: 4,
        fontSize: 16,
        threshold: 128,
        mode: 'binary' as const,
        ramp: DEFAULT_RAMP,
      }),
    ).toThrow('Cell size')
    expect(controller.getSnapshot().status).toBe('error')

    controller.reset()
    expect(controller.getSnapshot().metadata).toBeNull()
    expect(controller.getSnapshot().preview).toBeNull()
    expect(urls.revoked).toEqual(['blob:demo-1'])
    controller.cancel()
    expect(controller.getSnapshot().status).toBe('idle')
  })

  it('discards a stale preview after cancellation and disposes exactly once', async () => {
    const urls = createUrlApi()
    let release: (() => void) | undefined
    const adapter = createDemoProcessingAdapter({
      urlApi: urls,
      delay: () =>
        new Promise<void>((resolve) => {
          release = resolve
        }),
    })
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    const pending = controller.preview()
    controller.cancel()
    release?.()
    await pending
    expect(controller.getSnapshot().status).toBe('cancelled')
    expect(controller.getSnapshot().preview).toBeNull()
    expect(urls.created).toEqual([])

    controller.dispose()
    controller.dispose()
    expect(controller.getSnapshot().status).toBe('disposed')
  })

  it('invalidates a pending preview when settings change', async () => {
    const urls = createUrlApi()
    let release: (() => void) | undefined
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({
        urlApi: urls,
        delay: () =>
          new Promise<void>((resolve) => {
            release = resolve
          }),
      }),
    })
    await controller.loadDemo()
    const pending = controller.preview()
    controller.updateSettings({
      cellSize: 20,
      fontSize: 20,
      threshold: 200,
      mode: 'binary' as const,
      ramp: DEFAULT_RAMP,
    })
    release?.()
    await pending
    expect(controller.getSnapshot().status).toBe('ready')
    expect(controller.getSnapshot().settings.cellSize).toBe(20)
    expect(urls.created).toEqual([])
  })

  it('notifies subscribers of final disposal and keeps disposal idempotent', () => {
    const controller = createStudioController()
    const statuses: string[] = []
    controller.subscribe(() => statuses.push(controller.getSnapshot().status))
    controller.dispose()
    controller.dispose()
    expect(statuses.at(-1)).toBe('disposed')
  })

  it('keeps an invalid-settings error after an older preview settles', async () => {
    const urls = createUrlApi()
    let release: (() => void) | undefined
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({
        urlApi: urls,
        delay: () =>
          new Promise<void>((resolve) => {
            release = resolve
          }),
      }),
    })
    await controller.loadDemo()
    const pending = controller.preview()
    expect(() =>
      controller.updateSettings({
        cellSize: 4,
        fontSize: 16,
        threshold: 128,
        mode: 'binary' as const,
        ramp: DEFAULT_RAMP,
      }),
    ).toThrow('Cell size')
    release?.()
    await pending
    expect(controller.getSnapshot().status).toBe('error')
    expect(controller.getSnapshot().error).toContain('Cell size')
    expect(urls.created).toEqual([])
  })
})

function createUrlApi(): {
  readonly created: string[]
  readonly revoked: string[]
  createObjectURL(): string
  revokeObjectURL(url: string): void
} {
  const created: string[] = []
  const revoked: string[] = []
  return {
    created,
    revoked,
    createObjectURL: () => {
      const url = `blob:demo-${created.length + 1}`
      created.push(url)
      return url
    },
    revokeObjectURL: (url) => revoked.push(url),
  }
}
