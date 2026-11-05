import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import { until } from '../processing/testKit'
import type {
  ImportedResult,
  JobContext,
  ProcessingAdapter,
  ProcessingSession,
} from '../processing/types'
import { DEFAULT_SETTINGS } from '../processing/types'
import { createStudioController } from './controller'

interface CreatedImport {
  readonly session: ProcessingSession
  readonly result: ImportedResult
}

function gateway(): { readonly promise: Promise<void>; release(): void } {
  let release: () => void = () => undefined
  const promise = new Promise<void>((resolve) => {
    release = resolve
  })
  return { promise, release: () => release() }
}

/** `gate` runs after the base adapter has created a real owned import result. */
function trackedAdapter(gate?: (context: JobContext) => Promise<void>): {
  readonly adapter: ProcessingAdapter
  readonly disposed: string[]
  readonly created: CreatedImport[]
} {
  const base = createDemoProcessingAdapter()
  const disposed: string[] = []
  const created: CreatedImport[] = []
  const adapter: ProcessingAdapter = {
    createSession: () => base.createSession(),
    import: async (session, input, operation) => {
      const result = await base.import(session, input, operation)
      created.push({ session, result })
      await gate?.(operation)
      return {
        ...result,
        dispose: () => {
          disposed.push(`import-${operation.operationId}`)
          result.dispose()
        },
      }
    },
    preview: (session, source, settings, operation) =>
      base.preview(session, source, settings, operation),
    export: (session, source, format, settings, operation) =>
      base.export(session, source, format, settings, operation),
    cancel: (session, operationId) => base.cancel(session, operationId),
    dispose: (session) => base.dispose(session),
  }
  return { adapter, disposed, created }
}

describe('controller import ownership', () => {
  it('disposes the previous import only after a replacement is accepted', async () => {
    const { adapter, disposed } = trackedAdapter()
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    expect(disposed).toEqual([])
    await controller.loadDemo()
    expect(disposed).toEqual(['import-1'])
  })

  it('preserves the earlier import and preview when a replacement fails', async () => {
    const { adapter, disposed } = trackedAdapter()
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    await controller.preview()
    const preview = controller.getSnapshot().preview
    await expect(
      controller.import(new File(['x'], 'x.png', { type: 'image/png' })),
    ).rejects.toThrow('File conversion')
    expect(disposed).toEqual([])
    expect(controller.getSnapshot().preview).toEqual(preview)
    await controller.preview()
    expect(controller.getSnapshot().status).toBe('ready')
  })

  it('disposes the import on reset and on controller disposal exactly once', async () => {
    const { adapter, disposed } = trackedAdapter()
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    controller.reset()
    controller.reset()
    expect(disposed).toEqual(['import-1'])
    await controller.loadDemo()
    controller.dispose()
    controller.dispose()
    expect(disposed).toEqual(['import-1', 'import-2'])
  })

  it('disposes an import that arrives after cancellation', async () => {
    const gate = gateway()
    const { adapter, disposed, created } = trackedAdapter(() => gate.promise)
    const controller = createStudioController({ adapter })
    const pending = controller.loadDemo()
    await until(() => created.length === 1, 'owned import result')
    controller.cancel()
    expect(disposed).toEqual([])
    gate.release()
    await pending
    expect(disposed).toEqual(['import-1'])
    expect(controller.getSnapshot().metadata).toBeNull()
    const entry = created[0]
    if (entry === undefined) throw new Error('The import was not created.')
    await expect(
      adapter.preview(entry.session, entry.result.source, DEFAULT_SETTINGS, {
        operationId: 9,
        generation: 9,
      }),
    ).rejects.toThrow('does not belong')
  })

  it('disposes an import that arrives after the controller is disposed', async () => {
    const gate = gateway()
    const { adapter, disposed, created } = trackedAdapter(() => gate.promise)
    const controller = createStudioController({ adapter })
    const pending = controller.loadDemo()
    await until(() => created.length === 1, 'owned import result')
    controller.dispose()
    expect(disposed).toEqual([])
    gate.release()
    await pending
    expect(disposed).toEqual(['import-1'])
    expect(controller.getSnapshot().metadata).toBeNull()
  })
})
