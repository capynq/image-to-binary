import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import type {
  ExportFormat,
  ImportInput,
  ImportedResult,
  ImportedSource,
  PreviewArtifact,
  ProcessingAdapter,
  StudioSettings,
  StudioSnapshot,
} from '../processing/types'
import { DEFAULT_SETTINGS, cloneSettings, validateSettings } from '../processing/types'

export class StaleOperationError extends Error {
  constructor() {
    super('The processing operation became stale.')
    this.name = 'StaleOperationError'
  }
}

export interface StudioControllerOptions {
  readonly adapter?: ProcessingAdapter
  readonly settings?: StudioSettings
}

/** Stateful facade that owns one opaque adapter session and all revocable artifacts. */
export interface StudioController {
  readonly subscribe: (listener: () => void) => () => void
  readonly getSnapshot: () => StudioSnapshot
  loadDemo(): Promise<void>
  import(input: ImportInput): Promise<void>
  updateSettings(settings: StudioSettings): void
  preview(): Promise<void>
  exportDemo(format?: ExportFormat): Promise<Blob>
  reset(): void
  cancel(): void
  dispose(): void
}

interface ActiveOperation {
  readonly operationId: number
  readonly generation: number
}

/** Creates a controller with isolated session ownership and immutable snapshots. */
export function createStudioController(options: StudioControllerOptions = {}): StudioController {
  const adapter = options.adapter ?? createDemoProcessingAdapter()
  const session = adapter.createSession()
  const listeners = new Set<() => void>()
  let snapshot: StudioSnapshot = Object.freeze({
    status: 'idle',
    generation: 0,
    progress: 0,
    settings: cloneSettings(options.settings ?? DEFAULT_SETTINGS),
    metadata: null,
    preview: null,
    error: null,
  })
  validateSettings(snapshot.settings)

  let disposed = false
  let operationSequence = 0
  let activeOperation: ActiveOperation | null = null
  let source: ImportedSource | null = null
  let previewArtifact: PreviewArtifact | null = null

  const getSnapshot = (): StudioSnapshot => snapshot

  const emit = (): void => {
    for (const listener of listeners) {
      listener()
    }
  }

  const setSnapshot = (patch: Partial<StudioSnapshot>): void => {
    snapshot = Object.freeze({ ...snapshot, ...patch })
    emit()
  }

  const releasePreview = (): void => {
    previewArtifact?.dispose()
    previewArtifact = null
    setSnapshot({ preview: null })
  }

  const invalidate = (): void => {
    if (activeOperation !== null) {
      adapter.cancel(session, activeOperation.operationId)
      activeOperation = null
    }
    setSnapshot({ generation: snapshot.generation + 1 })
  }

  const beginOperation = (status: 'importing' | 'processing' | 'exporting'): ActiveOperation => {
    invalidate()
    const operation = { operationId: ++operationSequence, generation: snapshot.generation }
    activeOperation = operation
    setSnapshot({ status, progress: 0, error: null })
    return operation
  }

  const isCurrent = (operation: ActiveOperation): boolean =>
    !disposed &&
    activeOperation?.operationId === operation.operationId &&
    snapshot.generation === operation.generation

  const fail = (operation: ActiveOperation, error: unknown): void => {
    if (isCurrent(operation)) {
      activeOperation = null
      const message = error instanceof Error ? error.message : String(error)
      const suffix = source === null ? '' : ' Previous demo remains available.'
      setSnapshot({ status: 'error', error: `${message}${suffix}`, progress: 0 })
    }
  }

  const importSource = async (input: ImportInput): Promise<void> => {
    if (disposed) {
      throw new Error('Studio controller is disposed.')
    }
    const operation = beginOperation('importing')
    try {
      const imported: ImportedResult = await adapter.import(session, input, operation)
      if (!isCurrent(operation)) {
        return
      }
      source = imported.source
      activeOperation = null
      releasePreview()
      setSnapshot({
        status: 'ready',
        metadata: Object.freeze({ ...imported.metadata }),
        progress: 1,
      })
    } catch (error) {
      const current = isCurrent(operation)
      fail(operation, error)
      if (current) throw error
    }
  }

  const preview = async (): Promise<void> => {
    if (disposed) {
      throw new Error('Studio controller is disposed.')
    }
    if (source === null) {
      throw new Error('Import an image before previewing.')
    }
    const operation = beginOperation('processing')
    try {
      const artifact = await adapter.preview(session, source, snapshot.settings, operation)
      if (!isCurrent(operation)) {
        artifact.dispose()
        return
      }
      previewArtifact?.dispose()
      previewArtifact = artifact
      activeOperation = null
      setSnapshot({
        status: 'ready',
        preview: Object.freeze({
          url: artifact.url,
          width: artifact.width,
          height: artifact.height,
        }),
        progress: 1,
      })
    } catch (error) {
      const current = isCurrent(operation)
      fail(operation, error)
      if (current) throw error
    }
  }

  const controller: StudioController = {
    subscribe: (listener) => {
      if (disposed) {
        return () => undefined
      }
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    getSnapshot,
    loadDemo: () => importSource({ kind: 'demo' }),
    import: importSource,
    updateSettings: (settings) => {
      if (disposed) {
        throw new Error('Studio controller is disposed.')
      }
      try {
        validateSettings(settings)
      } catch (error) {
        invalidate()
        setSnapshot({
          status: 'error',
          error: error instanceof Error ? error.message : String(error),
        })
        throw error
      }
      invalidate()
      releasePreview()
      setSnapshot({
        settings: cloneSettings(settings),
        status: source === null ? 'idle' : 'ready',
        error: null,
      })
    },
    preview,
    exportDemo: async (format = 'text') => {
      if (disposed) {
        throw new Error('Studio controller is disposed.')
      }
      if (source === null) {
        throw new Error('Import an image before exporting.')
      }
      const operation = beginOperation('exporting')
      try {
        const result = await adapter.export(session, source, format, snapshot.settings, operation)
        if (!isCurrent(operation)) {
          throw new StaleOperationError()
        }
        activeOperation = null
        setSnapshot({ status: 'ready', progress: 1 })
        return result
      } catch (error) {
        fail(operation, error)
        throw error
      }
    },
    reset: () => {
      if (disposed) {
        return
      }
      invalidate()
      source = null
      releasePreview()
      setSnapshot({ status: 'idle', metadata: null, progress: 0, error: null })
    },
    cancel: () => {
      if (disposed || activeOperation === null) {
        return
      }
      invalidate()
      setSnapshot({ status: 'cancelled', progress: 0, error: null })
    },
    dispose: () => {
      if (disposed) {
        return
      }
      invalidate()
      releasePreview()
      adapter.dispose(session)
      disposed = true
      source = null
      setSnapshot({ status: 'disposed', metadata: null, progress: 0, error: null })
      listeners.clear()
    },
  }

  return controller
}
