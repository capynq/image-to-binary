import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import type {
  ExportArtifact,
  ExportFormat,
  ExportProgressListener,
  ImportedSource,
  JobContext,
  ProcessingAdapter,
  ProcessingSession,
  StudioSettings,
} from '../processing/types'
import { DEFAULT_SETTINGS } from '../processing/types'
import { createStudioController } from './controller'
import type { StudioController } from './controller'

interface ExportGate {
  readonly operationId: number
  readonly settings: StudioSettings
  readonly format: ExportFormat
  readonly onProgress: ExportProgressListener | undefined
  resolve(artifact: ExportArtifact): void
  reject(error: Error): void
}

interface Harness {
  readonly controller: StudioController
  readonly gates: ExportGate[]
  readonly artifacts: Array<{ readonly artifact: ExportArtifact; readonly disposals: number }>
}

interface HarnessOptions {
  /** When true the adapter rejects a cancelled export at once; otherwise it may still settle late. */
  readonly rejectOnCancel?: boolean
}

function harness(options: HarnessOptions = {}): Harness {
  const urls = {
    createObjectURL: () => 'blob:test',
    revokeObjectURL: () => undefined,
  }
  const base = createDemoProcessingAdapter({ urlApi: urls })
  const gates: ExportGate[] = []
  const artifacts: Harness['artifacts'] = []
  const adapter: ProcessingAdapter = {
    createSession: () => base.createSession(),
    import: (session: ProcessingSession, input, context: JobContext) =>
      base.import(session, input, context),
    preview: (session, source: ImportedSource, settings, context, kind) =>
      base.preview(session, source, settings, context, kind),
    export: (_session, _source, format, settings, context, onProgress) => {
      let resolve!: (artifact: ExportArtifact) => void
      let reject!: (error: Error) => void
      const promise = new Promise<ExportArtifact>((res, rej) => {
        resolve = res
        reject = rej
      })
      gates.push({
        operationId: context.operationId,
        settings: { ...settings },
        format,
        onProgress,
        resolve,
        reject,
      })
      return promise
    },
    cancel: (session, operationId) => {
      base.cancel(session, operationId)
      if (options.rejectOnCancel !== true) return
      for (const gate of gates) {
        if (gate.operationId === operationId)
          gate.reject(new Error('Processing operation was cancelled.'))
      }
    },
    dispose: (session) => base.dispose(session),
  }
  return { controller: createStudioController({ adapter }), gates, artifacts }
}

function makeArtifact(state: Harness): ExportArtifact {
  const record: { artifact: ExportArtifact | null; disposals: number } = {
    artifact: null,
    disposals: 0,
  }
  const artifact: ExportArtifact = {
    kind: 'blob',
    format: 'png',
    blob: new Blob([Uint8Array.of(1, 2, 3, 4, 5)], { type: 'image/png' }),
    byteLength: 5,
    dispose: () => {
      record.disposals += 1
    },
  }
  record.artifact = artifact
  state.artifacts.push({
    artifact,
    get disposals() {
      return record.disposals
    },
  })
  return artifact
}

function lastGate(state: Harness): ExportGate {
  const gate = state.gates.at(-1)
  if (gate === undefined) throw new Error('No export was started.')
  return gate
}

function disposalsOf(state: Harness, artifact: ExportArtifact): number {
  return state.artifacts.find((record) => record.artifact === artifact)?.disposals ?? -1
}

async function readyHarness(): Promise<Harness> {
  const state = harness()
  await state.controller.loadDemo()
  return state
}

async function readyExportFor(state: Harness): Promise<ExportArtifact> {
  const running = state.controller.startExport()
  const artifact = makeArtifact(state)
  lastGate(state).resolve(artifact)
  await running
  return artifact
}

describe('controller PNG export lifecycle', () => {
  it('publishes metadata only and keeps the Blob out of the snapshot', async () => {
    const state = await readyHarness()
    const running = state.controller.startExport()
    expect(state.controller.getSnapshot().export).toEqual({
      status: 'running',
      progress: 0,
      byteLength: null,
      error: null,
    })
    lastGate(state).onProgress?.(0.5)
    expect(state.controller.getSnapshot().export.progress).toBe(0.5)

    const artifact = makeArtifact(state)
    lastGate(state).resolve(artifact)
    await running

    const snapshot = state.controller.getSnapshot()
    expect(snapshot.export).toEqual({ status: 'ready', progress: 1, byteLength: 5, error: null })
    expect(snapshot.status).toBe('ready')
    expect(state.controller.readyExport()).toBe(artifact)
    expect(JSON.stringify(snapshot)).not.toContain('blob')
    expect(
      Object.values(snapshot.export).every((value) => typeof value !== 'object' || value === null),
    ).toBe(true)
  })

  it('uses the settings captured at start even when settings change afterwards', async () => {
    const state = await readyHarness()
    const running = state.controller.startExport()
    state.controller.updateSettings({ ...DEFAULT_SETTINGS, cellSize: 20 })

    expect(lastGate(state).settings).toEqual(DEFAULT_SETTINGS)
    expect(state.controller.getSnapshot().settings.cellSize).toBe(20)
    const artifact = makeArtifact(state)
    lastGate(state).resolve(artifact)
    await running

    expect(state.controller.getSnapshot().export.status).toBe('idle')
    expect(state.controller.readyExport()).toBeNull()
    expect(disposalsOf(state, artifact)).toBe(1)
  })

  it('a settings change during export supersedes it and publishes nothing', async () => {
    const state = await readyHarness()
    const running = state.controller.startExport()
    const artifact = makeArtifact(state)
    state.controller.updateSettings({ ...DEFAULT_SETTINGS, threshold: 10 })
    lastGate(state).resolve(artifact)
    await running
    expect(state.controller.getSnapshot().export.status).toBe('idle')
    expect(state.controller.readyExport()).toBeNull()
    expect(disposalsOf(state, artifact)).toBe(1)
  })

  it('a new preview during export supersedes it and the late result is discarded', async () => {
    const state = await readyHarness()
    const running = state.controller.startExport()
    const artifact = makeArtifact(state)
    await state.controller.preview()
    lastGate(state).resolve(artifact)
    await running

    expect(state.controller.getSnapshot().export.status).toBe('idle')
    expect(state.controller.readyExport()).toBeNull()
    expect(disposalsOf(state, artifact)).toBe(1)
  })

  it('a ready export survives a preview', async () => {
    const state = await readyHarness()
    const artifact = await readyExportFor(state)
    await state.controller.preview()
    expect(state.controller.getSnapshot().export.status).toBe('ready')
    expect(state.controller.readyExport()).toBe(artifact)
  })

  it.each([
    ['reset', (controller: StudioController) => controller.reset()],
    ['dispose', (controller: StudioController) => controller.dispose()],
    ['source replacement', (controller: StudioController) => controller.loadDemo()],
    ['cancel', (controller: StudioController) => controller.cancel()],
  ])('%s cancels a running export and discards its result', async (_name, trigger) => {
    const state = await readyHarness()
    const running = state.controller.startExport()
    const artifact = makeArtifact(state)
    const gate = lastGate(state)
    await trigger(state.controller)
    gate.resolve(artifact)
    await running

    expect(state.controller.getSnapshot().export.status).toBe('idle')
    expect(state.controller.readyExport()).toBeNull()
    expect(disposalsOf(state, artifact)).toBe(1)
  })

  it.each([
    [
      'settings change',
      (controller: StudioController) =>
        controller.updateSettings({ ...DEFAULT_SETTINGS, fontSize: 9 }),
    ],
    ['source replacement', (controller: StudioController) => controller.loadDemo()],
    ['reset', (controller: StudioController) => controller.reset()],
    ['dispose', (controller: StudioController) => controller.dispose()],
  ])('a ready export is invalidated by %s', async (_name, trigger) => {
    const state = await readyHarness()
    const artifact = await readyExportFor(state)
    await trigger(state.controller)

    expect(state.controller.getSnapshot().export.status).toBe('idle')
    expect(state.controller.readyExport()).toBeNull()
    expect(disposalsOf(state, artifact)).toBe(1)
  })

  it('a prompt adapter rejection of a superseded export is not reported as an error', async () => {
    const state = harness({ rejectOnCancel: true })
    await state.controller.loadDemo()
    const running = state.controller.startExport()
    state.controller.updateSettings({ ...DEFAULT_SETTINGS, cellSize: 7 })
    await expect(running).resolves.toBeUndefined()
    expect(state.controller.getSnapshot().status).toBe('ready')
    expect(state.controller.getSnapshot().export.status).toBe('idle')
  })

  it('a failed export is typed, publishes nothing and rejects while current', async () => {
    const state = await readyHarness()
    const running = state.controller.startExport()
    lastGate(state).reject(new Error('The encoder ran out of memory.'))

    await expect(running).rejects.toThrow('ran out of memory')
    const snapshot = state.controller.getSnapshot()
    expect(snapshot.export).toEqual({
      status: 'failed',
      progress: 0,
      byteLength: null,
      error: 'The encoder ran out of memory.',
    })
    expect(snapshot.status).toBe('error')
    expect(state.controller.readyExport()).toBeNull()
  })

  it('rejects an export before any source is imported without changing state', async () => {
    const state = harness()
    const before = state.controller.getSnapshot()
    await expect(state.controller.startExport()).rejects.toThrow('Import an image')
    expect(state.controller.getSnapshot()).toBe(before)
    expect(state.gates).toHaveLength(0)
  })
})

describe('controller export format', () => {
  it('starts the adapter with the chosen text format and keeps the text artifact ready', async () => {
    const state = await readyHarness()
    const running = state.controller.startExport('text')
    expect(lastGate(state).format).toBe('text')
    const artifact: ExportArtifact = {
      kind: 'blob',
      format: 'text',
      blob: new Blob([Uint8Array.of(0x41, 0x0a)], { type: 'text/plain;charset=utf-8' }),
      byteLength: 2,
      dispose: () => undefined,
    }
    lastGate(state).resolve(artifact)
    await running
    expect(state.controller.readyExport()).toBe(artifact)
    expect(state.controller.getSnapshot().export).toEqual({
      status: 'ready',
      progress: 1,
      byteLength: 2,
      error: null,
    })
  })

  it('defaults to PNG when no format is given', async () => {
    const state = await readyHarness()
    const running = state.controller.startExport()
    expect(lastGate(state).format).toBe('png')
    lastGate(state).resolve(makeArtifact(state))
    await running
  })
})
