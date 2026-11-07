import { describe, expect, it } from 'vitest'
import { createStudioController } from '../state/controller'

describe('demo text export', () => {
  it('ends every row with an LF, so the demo text follows the text contract', async () => {
    const controller = createStudioController()
    await controller.loadDemo()
    await controller.startExport('text')
    const artifact = controller.readyExport()
    if (artifact?.kind !== 'blob') throw new Error('expected a blob text export')
    const text = await artifact.blob.text()
    controller.dispose()
    expect(text.endsWith('\n')).toBe(true)
    expect(text.includes('\r')).toBe(false)
    const rows = text.split('\n')
    expect(rows.pop()).toBe('')
    expect(rows.length).toBeGreaterThan(0)
    expect(new Set(rows.map((row) => row.length)).size).toBe(1)
    expect(artifact.byteLength).toBe(new TextEncoder().encode(text).byteLength)
  })
})
