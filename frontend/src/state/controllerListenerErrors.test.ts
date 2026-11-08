import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import { DEFAULT_SETTINGS } from '../processing/types'
import { createStudioController } from './controller'

function countingUrls() {
  let next = 0
  const revoked: string[] = []
  return {
    revoked,
    api: {
      createObjectURL: () => {
        next += 1
        return `blob:${next}`
      },
      revokeObjectURL: (url: string) => {
        revoked.push(url)
      },
    },
  }
}

describe('controller listener errors', () => {
  it('a throwing listener does not stop the other listeners', async () => {
    const urls = countingUrls()
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({ urlApi: urls.api }),
    })
    await controller.loadDemo()
    let armed = true
    let secondCalls = 0
    controller.subscribe(() => {
      if (armed) throw new Error('listener boom')
    })
    controller.subscribe(() => {
      secondCalls += 1
    })

    armed = true
    expect(() => controller.updateSettings({ ...DEFAULT_SETTINGS, cellSize: 20 })).toThrow(
      'listener boom',
    )
    expect(secondCalls).toBeGreaterThan(0)
    expect(controller.getSnapshot().settings.cellSize).toBe(20)
  })

  it('a settings change keeps the stale artwork when a listener throws, and a replacement releases it once', async () => {
    const urls = countingUrls()
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({ urlApi: urls.api }),
    })
    await controller.loadDemo()
    await controller.preview()
    const previewUrl = controller.getSnapshot().preview?.url
    let armed = false
    controller.subscribe(() => {
      if (armed) throw new Error('listener boom')
    })

    armed = true
    expect(() => controller.updateSettings({ ...DEFAULT_SETTINGS, cellSize: 15 })).toThrow(
      'listener boom',
    )
    expect(previewUrl).toBeDefined()
    expect(urls.revoked).toEqual([])
    expect(controller.getSnapshot().preview?.url).toBe(previewUrl)
    armed = false
    await controller.preview()
    expect(urls.revoked.filter((url) => url === previewUrl)).toHaveLength(1)
  })

  it('a preview whose publish listener throws surfaces the error once, keeps the new state and revokes the old artifact once', async () => {
    const urls = countingUrls()
    const controller = createStudioController({
      adapter: createDemoProcessingAdapter({ urlApi: urls.api }),
    })
    await controller.loadDemo()
    await controller.preview()
    const first = controller.getSnapshot().preview?.url
    let armed = false
    controller.subscribe(() => {
      if (armed && controller.getSnapshot().status === 'ready') throw new Error('listener boom')
    })

    armed = true
    await expect(controller.preview()).rejects.toThrow('listener boom')
    const snapshot = controller.getSnapshot()
    expect(snapshot.status).toBe('ready')
    expect(snapshot.error).toBeNull()
    expect(snapshot.preview?.url).not.toBe(first)
    expect(urls.revoked.filter((url) => url === first)).toHaveLength(1)

    armed = false
    controller.dispose()
    const counts = new Map<string, number>()
    for (const url of urls.revoked) counts.set(url, (counts.get(url) ?? 0) + 1)
    expect([...counts.values()].every((count) => count === 1)).toBe(true)
  })

  it('an import whose publish listener throws keeps the new source and reports the error once', async () => {
    const controller = createStudioController()
    let armed = false
    controller.subscribe(() => {
      if (armed && controller.getSnapshot().status === 'ready') throw new Error('listener boom')
    })

    armed = true
    await expect(controller.loadDemo()).rejects.toThrow('listener boom')
    expect(controller.getSnapshot().metadata?.name).toBe('Demo fixture')
    expect(controller.getSnapshot().status).toBe('ready')
  })
})
