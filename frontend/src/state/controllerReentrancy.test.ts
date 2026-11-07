import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import type {
  ExportArtifact,
  ExportProgressListener,
  ProcessingAdapter,
  StudioSettings,
} from '../processing/types'
import { DEFAULT_SETTINGS } from '../processing/types'
import { createStudioController } from './controller'
import type { StudioController } from './controller'

interface Gate {
  readonly settings: StudioSettings
  readonly onProgress: ExportProgressListener | undefined
  resolve(artifact: ExportArtifact): void
}

function gatedController() {
  const base = createDemoProcessingAdapter({
    urlApi: { createObjectURL: () => 'blob:test', revokeObjectURL: () => undefined },
  })
  const gates: Gate[] = []
  const adapter: ProcessingAdapter = {
    createSession: () => base.createSession(),
    import: (session, input, context) => base.import(session, input, context),
    preview: (session, source, settings, context, kind) =>
      base.preview(session, source, settings, context, kind),
    export: (_session, _source, _format, settings, _context, onProgress) =>
      new Promise<ExportArtifact>((resolve) => {
        gates.push({ settings: { ...settings }, onProgress, resolve })
      }),
    cancel: (session, operationId) => base.cancel(session, operationId),
    dispose: (session) => base.dispose(session),
  }
  return { controller: createStudioController({ adapter }), gates }
}

function artifactWith(log: { disposals: number }): ExportArtifact {
  return {
    kind: 'blob',
    blob: new Blob([Uint8Array.of(7)], { type: 'image/png' }),
    byteLength: 1,
    dispose: () => {
      log.disposals += 1
    },
  }
}

/** Subscribes a listener that runs `action` once, from inside the first commit it observes. */
function once(controller: StudioController, action: () => void): void {
  let armed = true
  controller.subscribe(() => {
    if (!armed) return
    armed = false
    action()
  })
}

describe('reentrant listeners and export settings', () => {
  it('an export started from a listener during a settings commit uses the new settings', async () => {
    const { controller, gates } = gatedController()
    await controller.loadDemo()
    once(controller, () => {
      void controller.startExport().catch(() => undefined)
    })

    controller.updateSettings({ ...DEFAULT_SETTINGS, cellSize: 20 })

    expect(gates).toHaveLength(1)
    expect(gates[0]?.settings.cellSize).toBe(20)
    const log = { disposals: 0 }
    gates[0]?.resolve(artifactWith(log))
    await Promise.resolve()
    await Promise.resolve()
    expect(controller.getSnapshot().export.status).toBe('ready')
    expect(controller.readyExport()).not.toBeNull()
  })

  it('old-settings bytes from a superseded export never publish as ready', async () => {
    const { controller, gates } = gatedController()
    await controller.loadDemo()
    const oldRun = controller.startExport()
    const oldLog = { disposals: 0 }
    once(controller, () => {
      void controller.startExport().catch(() => undefined)
    })

    controller.updateSettings({ ...DEFAULT_SETTINGS, cellSize: 20 })
    const oldGate = gates[0]
    const newGate = gates[1]
    expect(oldGate?.settings.cellSize).toBe(DEFAULT_SETTINGS.cellSize)
    expect(newGate?.settings.cellSize).toBe(20)

    oldGate?.resolve(artifactWith(oldLog))
    await oldRun
    expect(oldLog.disposals).toBe(1)
    expect(controller.readyExport()).toBeNull()
    expect(controller.getSnapshot().export.status).toBe('running')

    const newLog = { disposals: 0 }
    const current = artifactWith(newLog)
    newGate?.resolve(current)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(controller.readyExport()).toBe(current)
    expect(newLog.disposals).toBe(0)
  })

  it('a reset from a listener during a settings commit leaves no export running', async () => {
    const { controller, gates } = gatedController()
    await controller.loadDemo()
    once(controller, () => {
      controller.reset()
    })
    controller.updateSettings({ ...DEFAULT_SETTINGS, cellSize: 20 })
    expect(gates).toHaveLength(0)
    expect(controller.getSnapshot().metadata).toBeNull()
  })

  it('a settings change from a listener during export start prevents the adapter call', async () => {
    const { controller, gates } = gatedController()
    await controller.loadDemo()
    once(controller, () => {
      controller.updateSettings({ ...DEFAULT_SETTINGS, threshold: 9 })
    })

    await controller.startExport()
    expect(gates).toHaveLength(0)
    expect(controller.getSnapshot().export.status).toBe('idle')
    expect(controller.readyExport()).toBeNull()
  })
})
