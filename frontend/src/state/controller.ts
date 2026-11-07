import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import type {
  ExportArtifact,
  ExportFormat,
  ExportSnapshot,
  ImportInput,
  ImportedResult,
  PreviewArtifact,
  PreviewKind,
  PreviewSnapshot,
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
  /**
   * Renders `kind` (default `artwork`) and publishes it only if the operation is still current.
   * Rejects an unknown kind before any state changes.
   */
  preview(kind?: PreviewKind): Promise<void>
  exportDemo(format?: ExportFormat): Promise<Blob>
  /**
   * Encodes the accepted source with the settings in effect when the export starts. A newer
   * preview, settings change, source change, reset or disposal supersedes it without publishing.
   * Listeners run after each snapshot is complete, so a listener may call back into the
   * controller; such a call is judged against the snapshot that was just committed.
   */
  startExport(): Promise<void>
  /** The ready PNG export, or null. Kept out of snapshots; the download phase reads it. */
  readyExport(): ExportArtifact | null
  reset(): void
  cancel(): void
  dispose(): void
}

type OperationKind = 'import' | 'preview' | 'export' | 'textExport'

interface ActiveOperation {
  readonly operationId: number
  readonly generation: number
  readonly kind: OperationKind
}

interface Disposable {
  dispose(): void
}

const IDLE_EXPORT: ExportSnapshot = Object.freeze({
  status: 'idle',
  progress: 0,
  byteLength: null,
  error: null,
})

const RUNNING_EXPORT: ExportSnapshot = Object.freeze({
  status: 'running',
  progress: 0,
  byteLength: null,
  error: null,
})

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
    importStage: null,
    preview: null,
    original: null,
    export: IDLE_EXPORT,
    error: null,
  })
  validateSettings(snapshot.settings)

  let disposed = false
  let operationSequence = 0
  let activeOperation: ActiveOperation | null = null
  let ownedImport: ImportedResult | null = null
  let readyExport: ExportArtifact | null = null
  const artifacts: Record<PreviewKind, PreviewArtifact | null> = { artwork: null, original: null }

  const getSnapshot = (): StudioSnapshot => snapshot

  /**
   * Publishes one complete snapshot, then notifies listeners. Every field of a transition is
   * already in place when the first listener runs, so a reentrant call sees a consistent state.
   * `discards` are resources no longer referenced by the snapshot. Every listener runs and every
   * discard is released even when a listener throws; the first listener error is then rethrown
   * once, after cleanup. The committed state is never rolled back.
   */
  const commit = (
    patch: Partial<StudioSnapshot>,
    discards: ReadonlyArray<Disposable | null>,
  ): void => {
    snapshot = Object.freeze({ ...snapshot, ...patch })
    let listenerFailure: { readonly error: unknown } | null = null
    for (const listener of listeners) {
      try {
        listener()
      } catch (error) {
        listenerFailure ??= { error }
      }
    }
    for (const resource of discards) {
      disposeQuietly(resource)
    }
    if (listenerFailure !== null) throw listenerFailure.error
  }

  /** Cancels the running operation without publishing; returns its kind, if any. */
  const supersede = (): OperationKind | null => {
    const running = activeOperation
    if (running === null) return null
    activeOperation = null
    adapter.cancel(session, running.operationId)
    return running.kind
  }

  const exportIdleIf = (cancelled: OperationKind | null): Partial<StudioSnapshot> =>
    cancelled === 'export' ? { export: IDLE_EXPORT } : {}

  const beginOperation = (
    status: 'importing' | 'processing' | 'exporting',
    kind: OperationKind,
    extra: Partial<StudioSnapshot> = {},
    discards: ReadonlyArray<Disposable | null> = [],
  ): ActiveOperation => {
    const cancelled = supersede()
    const operation: ActiveOperation = {
      operationId: ++operationSequence,
      generation: snapshot.generation + 1,
      kind,
    }
    activeOperation = operation
    commit(
      {
        ...exportIdleIf(cancelled),
        importStage: null,
        ...extra,
        generation: operation.generation,
        status,
        progress: 0,
        error: null,
      },
      discards,
    )
    return operation
  }

  const isCurrent = (operation: ActiveOperation): boolean =>
    !disposed &&
    activeOperation?.operationId === operation.operationId &&
    snapshot.generation === operation.generation

  const fail = (
    operation: ActiveOperation,
    error: unknown,
    extra: Partial<StudioSnapshot> = {},
  ): void => {
    if (!isCurrent(operation)) return
    activeOperation = null
    const message = errorText(error)
    const separator = /[.!?…]$/.test(message) ? ' ' : '. '
    const text =
      ownedImport === null || message === ''
        ? message
        : `${message}${separator}Previous image remains available.`
    commit({ status: 'error', error: text, progress: 0, importStage: null, ...extra }, [])
  }

  /** Swaps the source and clears every source-derived artifact in one commit. */
  const replaceOwned = (next: ImportedResult | null, patch: Partial<StudioSnapshot>): void => {
    const discards: Array<Disposable | null> = [
      artifacts.artwork,
      artifacts.original,
      ownedImport,
      readyExport,
    ]
    artifacts.artwork = null
    artifacts.original = null
    ownedImport = next
    readyExport = null
    commit(
      { ...patch, importStage: null, preview: null, original: null, export: IDLE_EXPORT },
      discards,
    )
  }

  const importSource = async (input: ImportInput): Promise<void> => {
    if (disposed) {
      throw new Error('Studio controller is disposed.')
    }
    const operation = beginOperation('importing', 'import')
    let imported: ImportedResult
    try {
      imported = await adapter.import(session, input, operation, (stage) => {
        if (isCurrent(operation)) commit({ importStage: stage }, [])
      })
    } catch (error) {
      const current = isCurrent(operation)
      fail(operation, error)
      if (current) throw error
      return
    }
    if (!isCurrent(operation)) {
      disposeQuietly(imported)
      return
    }
    activeOperation = null
    replaceOwned(imported, {
      status: 'ready',
      metadata: Object.freeze({ ...imported.metadata }),
      progress: 1,
    })
  }

  const preview = async (kind: PreviewKind = 'artwork'): Promise<void> => {
    if (disposed) {
      throw new Error('Studio controller is disposed.')
    }
    if (kind !== 'artwork' && kind !== 'original') {
      throw new RangeError('Unknown preview kind.')
    }
    const owned = ownedImport
    if (owned === null) {
      throw new Error('Import an image before previewing.')
    }
    const operation = beginOperation('processing', 'preview')
    let artifact: PreviewArtifact
    try {
      artifact = await adapter.preview(session, owned.source, snapshot.settings, operation, kind)
    } catch (error) {
      const current = isCurrent(operation)
      fail(operation, error)
      if (current) throw error
      return
    }
    if (!isCurrent(operation)) {
      disposeQuietly(artifact)
      return
    }
    const previous = artifacts[kind]
    artifacts[kind] = artifact
    activeOperation = null
    commit(
      {
        ...viewPatch(
          kind,
          Object.freeze({
            url: artifact.url,
            width: artifact.width,
            height: artifact.height,
          }),
        ),
        status: 'ready',
        progress: 1,
      },
      [previous],
    )
  }

  const startExport = async (): Promise<void> => {
    if (disposed) {
      throw new Error('Studio controller is disposed.')
    }
    const owned = ownedImport
    if (owned === null) {
      throw new Error('Import an image before exporting.')
    }
    const previousReady = readyExport
    readyExport = null
    const operation = beginOperation('exporting', 'export', { export: RUNNING_EXPORT }, [
      previousReady,
    ])
    if (!isCurrent(operation)) return
    const settings = snapshot.settings
    let artifact: ExportArtifact
    try {
      artifact = await adapter.export(
        session,
        owned.source,
        'png',
        settings,
        operation,
        (fraction) => {
          if (isCurrent(operation)) {
            commit({ export: { ...snapshot.export, progress: fraction } }, [])
          }
        },
      )
    } catch (error) {
      const current = isCurrent(operation)
      fail(operation, error, {
        export: { status: 'failed', progress: 0, byteLength: null, error: errorText(error) },
      })
      if (current) throw error
      return
    }
    if (!isCurrent(operation)) {
      disposeQuietly(artifact)
      return
    }
    activeOperation = null
    readyExport = artifact
    commit(
      {
        status: 'ready',
        progress: 1,
        export: { status: 'ready', progress: 1, byteLength: artifact.byteLength, error: null },
      },
      [],
    )
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
        const cancelled = supersede()
        commit(
          {
            generation: snapshot.generation + 1,
            ...exportIdleIf(cancelled),
            importStage: null,
            status: 'error',
            error: error instanceof Error ? error.message : String(error),
          },
          [],
        )
        throw error
      }
      supersede()
      const discards: Array<Disposable | null> = [artifacts.artwork, readyExport]
      artifacts.artwork = null
      readyExport = null
      commit(
        {
          settings: cloneSettings(settings),
          generation: snapshot.generation + 1,
          status: ownedImport === null ? 'idle' : 'ready',
          importStage: null,
          preview: null,
          export: IDLE_EXPORT,
          error: null,
        },
        discards,
      )
    },
    preview,
    exportDemo: async (format = 'text') => {
      if (disposed) {
        throw new Error('Studio controller is disposed.')
      }
      const owned = ownedImport
      if (owned === null) {
        throw new Error('Import an image before exporting.')
      }
      const settings = snapshot.settings
      const operation = beginOperation('exporting', 'textExport')
      let artifact: ExportArtifact
      try {
        artifact = await adapter.export(session, owned.source, format, settings, operation)
      } catch (error) {
        fail(operation, error)
        throw error
      }
      if (!isCurrent(operation)) {
        disposeQuietly(artifact)
        throw new StaleOperationError()
      }
      activeOperation = null
      commit({ status: 'ready', progress: 1 }, [])
      return artifact.blob
    },
    startExport,
    readyExport: () => readyExport,
    reset: () => {
      if (disposed) {
        return
      }
      supersede()
      replaceOwned(null, {
        generation: snapshot.generation + 1,
        status: 'idle',
        metadata: null,
        progress: 0,
        error: null,
      })
    },
    cancel: () => {
      if (disposed || activeOperation === null) {
        return
      }
      const cancelled = supersede()
      commit(
        {
          generation: snapshot.generation + 1,
          status: 'cancelled',
          importStage: null,
          progress: 0,
          error: null,
          ...exportIdleIf(cancelled),
        },
        [],
      )
    },
    dispose: () => {
      if (disposed) {
        return
      }
      supersede()
      const generation = snapshot.generation + 1
      disposed = true
      try {
        replaceOwned(null, {
          generation,
          status: 'disposed',
          metadata: null,
          progress: 0,
          error: null,
        })
      } finally {
        adapter.dispose(session)
        listeners.clear()
      }
    },
  }

  return controller
}

function errorText(error: unknown): string {
  return (error instanceof Error ? error.message : String(error)).trimEnd()
}

function disposeQuietly(resource: Disposable | null): void {
  try {
    resource?.dispose()
  } catch {}
}

function viewPatch(kind: PreviewKind, view: PreviewSnapshot | null): Partial<StudioSnapshot> {
  return kind === 'artwork' ? { preview: view } : { original: view }
}
