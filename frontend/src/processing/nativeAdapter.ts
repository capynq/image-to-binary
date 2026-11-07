import { exportFilename } from '../export/downloadName'
import { defaultUrlApi, isDemoFixture } from './demoAdapter'
import type { NativeBridge, NativeChannel } from './nativeBridge'
import { isCount, malformedReply, nativeError } from './nativeErrors'
import { parsePreviewFrame } from './nativeFrame'
import { renderPreviewPng } from './previewRenderer'
import type { PreviewRenderer } from './previewRenderer'
import type {
  ExportArtifact,
  ExportProgressListener,
  ImportedResult,
  ImportedSource,
  ImportInput,
  JobContext,
  NativeFileInput,
  PreviewArtifact,
  PreviewKind,
  PreviewProgressListener,
  ProcessingAdapter,
  ProcessingSession,
  StudioMetadata,
  StudioSettings,
  UrlApi,
} from './types'
import { isOutputKindName } from './protocol'
import { ExportCancelledError, previewParametersOf, validateSettings } from './types'
import { ProcessingWorkerError } from './workerClient'
import type { PreviewPixels } from './workerClient'

export class NativeUnsupportedInputError extends Error {
  constructor() {
    super('Only files chosen through the native dialog or a native drop can be imported.')
    this.name = 'NativeUnsupportedInputError'
  }
}

export interface NativeAdapterOptions {
  readonly bridge: NativeBridge
  readonly renderPreview?: PreviewRenderer
  readonly urlApi?: UrlApi
}

interface NativeSourceRecord {
  readonly name: string
  released: boolean
}

interface NativeOperation {
  /** Generation the shell cancels by. */
  readonly generation: number
  cancelled: boolean
}

interface NativeSessionState {
  disposed: boolean
  /** The one source the shell holds. Only a successful, uncancelled import becomes it. */
  current: NativeSourceRecord | null
  readonly sources: WeakMap<ImportedSource, NativeSourceRecord>
  readonly operations: Map<number, NativeOperation>
  /** Settles when the latest cancel has reached the shell; later commands wait for it. */
  pendingCancel: Promise<void>
}

interface ImportReply {
  readonly name: string
  readonly width: number
  readonly height: number
  readonly inputBytes: number | null
}

interface SinkReply {
  readonly id: number
  readonly name: string
}

interface ExportReply {
  readonly byteLength: number
}

interface ProgressMessage {
  readonly generation: number
  readonly done: number
  readonly total: number
}

const NO_CANCEL: Promise<void> = Promise.resolve()

/**
 * Adapter that runs import, preview and export in the desktop shell through the typed native
 * commands. The page holds only opaque handles; preview frames are validated and turned into the
 * same PNG object URL as the browser path, and PNG or text exports stream into a file the shell
 * commits with the same atomic sink.
 *
 * Ordering: a cancel is awaited before any later command of the session, so the shell has
 * applied it first. An import that was cancelled is never adopted as the current source.
 */
export function createNativeProcessingAdapter(options: NativeAdapterOptions): ProcessingAdapter {
  const { bridge } = options
  const renderPreview = options.renderPreview ?? renderPreviewPng
  const urlApi = options.urlApi ?? defaultUrlApi()
  const sessions = new WeakMap<ProcessingSession, NativeSessionState>()

  const assertSession = (session: ProcessingSession): NativeSessionState => {
    const state = sessions.get(session)
    if (state === undefined) {
      throw new Error('Processing session does not belong to this adapter.')
    }
    return state
  }
  const assertActive = (state: NativeSessionState): void => {
    if (state.disposed) throw new Error('Processing session is disposed.')
  }
  const sourceOf = (state: NativeSessionState, source: ImportedSource): NativeSourceRecord => {
    const record = state.sources.get(source)
    if (record === undefined || record.released || state.current !== record) {
      throw new Error('Imported source does not belong to this processing session.')
    }
    return record
  }
  const callQuietly = (command: string, args?: Record<string, unknown>): void => {
    bridge.invoke(command, args).catch(() => undefined)
  }
  const releaseHandle = (id: number): Promise<void> =>
    bridge.invoke('release_handle', { id }).then(
      () => undefined,
      () => undefined,
    )
  const beginOperation = (state: NativeSessionState, context: JobContext): NativeOperation => {
    const operation: NativeOperation = { generation: context.generation, cancelled: false }
    state.operations.set(context.operationId, operation)
    return operation
  }
  const progressChannel = (
    generation: number,
    onProgress: ((fraction: number) => void) | undefined,
  ): NativeChannel<ProgressMessage> => {
    const channel = bridge.channel<ProgressMessage>()
    channel.onmessage = (message) => {
      if (message.generation !== generation || onProgress === undefined) return
      if (message.total <= 0) return
      onProgress(Math.min(1, Math.max(0, message.done / message.total)))
    }
    return channel
  }
  const adopt = (
    state: NativeSessionState,
    reply: ImportReply,
    source: StudioMetadata['source'],
  ): ImportedResult => {
    if (state.disposed) {
      callQuietly('reset')
      throw new Error('Processing session is disposed.')
    }
    const record: NativeSourceRecord = { name: reply.name, released: false }
    const handle = {} as ImportedSource
    state.sources.set(handle, record)
    state.current = record
    const metadata: StudioMetadata =
      source === 'demo'
        ? { name: reply.name, width: reply.width, height: reply.height, source }
        : {
            name: reply.name,
            width: reply.width,
            height: reply.height,
            source,
            inputBytes: reply.inputBytes ?? 0,
          }
    return {
      source: handle,
      metadata,
      dispose(): void {
        if (record.released) return
        record.released = true
        state.sources.delete(handle)
        if (state.current === record) {
          state.current = null
          callQuietly('reset')
        }
      },
    }
  }
  const runImport = async (
    state: NativeSessionState,
    context: JobContext,
    command: 'import_demo' | 'import_source',
    args: Record<string, unknown>,
    releaseId: number | null,
    source: StudioMetadata['source'],
  ): Promise<ImportedResult> => {
    const operation = beginOperation(state, context)
    let reply: ImportReply
    try {
      await state.pendingCancel
      reply = await invokeImport(bridge, command, args)
    } finally {
      state.operations.delete(context.operationId)
      if (releaseId !== null) await releaseHandle(releaseId)
    }
    if (operation.cancelled) throw cancelledError()
    return adopt(state, reply, source)
  }

  return {
    createSession(): ProcessingSession {
      const session = {} as ProcessingSession
      sessions.set(session, {
        disposed: false,
        current: null,
        sources: new WeakMap<ImportedSource, NativeSourceRecord>(),
        operations: new Map<number, NativeOperation>(),
        pendingCancel: NO_CANCEL,
      })
      return session
    },
    async import(session, input: ImportInput, context): Promise<ImportedResult> {
      const state = assertSession(session)
      assertActive(state)
      if (isDemoFixture(input)) {
        return runImport(
          state,
          context,
          'import_demo',
          { generation: context.generation },
          null,
          'demo',
        )
      }
      if (!isNativeFile(input)) {
        throw new NativeUnsupportedInputError()
      }
      return runImport(
        state,
        context,
        'import_source',
        { fileId: input.id, generation: context.generation },
        input.id,
        'file',
      )
    },
    async preview(
      session,
      source,
      settings: StudioSettings,
      context,
      kind: PreviewKind = 'artwork',
      onProgress?: PreviewProgressListener,
    ): Promise<PreviewArtifact> {
      const state = assertSession(session)
      assertActive(state)
      sourceOf(state, source)
      if (kind !== 'artwork' && kind !== 'original') {
        throw new RangeError('Unknown preview kind.')
      }
      if (kind === 'artwork') validateSettings(settings)
      const generation = context.generation
      const operation = beginOperation(state, context)
      try {
        let body: unknown
        try {
          await state.pendingCancel
          body = await bridge.invoke('start_preview', {
            kind,
            settings: kind === 'artwork' ? previewParametersOf(settings) : null,
            generation,
            progress: progressChannel(generation, onProgress),
          })
        } catch (error) {
          throw nativeError(error)
        }
        if (operation.cancelled) throw cancelledError()
        const frame = parsePreviewFrame(body, generation)
        const pixels: PreviewPixels = {
          width: frame.width,
          height: frame.height,
          pixels: frame.pixels,
        }
        return await artifactFor(pixels, renderPreview, urlApi)
      } finally {
        state.operations.delete(context.operationId)
      }
    },
    async export(
      session,
      source,
      format,
      settings,
      context,
      onProgress?: ExportProgressListener,
    ): Promise<ExportArtifact> {
      const state = assertSession(session)
      assertActive(state)
      const record = sourceOf(state, source)
      if (!isOutputKindName(format)) {
        throw new RangeError('Export format must be png or text.')
      }
      validateSettings(settings)
      const generation = context.generation
      const operation = beginOperation(state, context)
      let sink: SinkReply | null = null
      try {
        let chosen: unknown
        try {
          await state.pendingCancel
          chosen = await bridge.invoke('save_image_dialog', {
            suggestedName: exportFilename(record.name, format),
            outputKind: format,
          })
        } catch (error) {
          throw nativeError(error)
        }
        if (chosen === null) throw new ExportCancelledError()
        sink = readSinkReply(chosen)
        if (operation.cancelled) throw cancelledError()
        let reply: unknown
        try {
          reply = await bridge.invoke('export_file', {
            settings: previewParametersOf(settings),
            sinkId: sink.id,
            generation,
            outputKind: format,
            progress: progressChannel(generation, onProgress),
          })
        } catch (error) {
          throw nativeError(error)
        }
        return savedArtifact(sink.name, readExportReply(reply).byteLength)
      } finally {
        state.operations.delete(context.operationId)
        if (sink !== null) await releaseHandle(sink.id)
      }
    },
    cancel(session, operationId): void {
      const state = assertSession(session)
      if (state.disposed) return
      const operation = state.operations.get(operationId)
      if (operation === undefined) return
      operation.cancelled = true
      state.pendingCancel = bridge.invoke('cancel', { generation: operation.generation }).then(
        () => undefined,
        () => undefined,
      )
    },
    dispose(session): void {
      const state = assertSession(session)
      if (state.disposed) return
      state.disposed = true
      for (const operation of state.operations.values()) {
        operation.cancelled = true
        callQuietly('cancel', { generation: operation.generation })
      }
      state.operations.clear()
      const held = state.current
      state.current = null
      if (held !== null) {
        held.released = true
        callQuietly('reset')
      }
    },
  }
}

function isNativeFile(input: ImportInput): input is NativeFileInput {
  return (
    typeof input === 'object' && input !== null && 'kind' in input && input.kind === 'native-file'
  )
}

function cancelledError(): ProcessingWorkerError {
  return new ProcessingWorkerError('cancelled', 'The processing operation was cancelled.')
}

async function invokeImport(
  bridge: NativeBridge,
  command: 'import_demo' | 'import_source',
  args: Record<string, unknown>,
): Promise<ImportReply> {
  let reply: unknown
  try {
    reply = await bridge.invoke(command, args)
  } catch (error) {
    throw nativeError(error)
  }
  return readImportReply(reply)
}

function readImportReply(reply: unknown): ImportReply {
  if (typeof reply !== 'object' || reply === null) throw malformedReply()
  const { name, width, height, inputBytes } = reply as Record<string, unknown>
  if (typeof name !== 'string' || !isCount(width) || !isCount(height)) throw malformedReply()
  if (inputBytes !== undefined && !isCount(inputBytes)) throw malformedReply()
  return {
    name,
    width,
    height,
    inputBytes: typeof inputBytes === 'number' ? inputBytes : null,
  }
}

function readSinkReply(reply: unknown): SinkReply {
  if (typeof reply !== 'object' || reply === null) throw malformedReply()
  const { id, name } = reply as Record<string, unknown>
  if (!isCount(id) || typeof name !== 'string') throw malformedReply()
  return { id, name }
}

function readExportReply(reply: unknown): ExportReply {
  if (typeof reply !== 'object' || reply === null) throw malformedReply()
  const { byteLength } = reply as Record<string, unknown>
  if (!isCount(byteLength)) throw malformedReply()
  return { byteLength }
}

async function artifactFor(
  pixels: PreviewPixels,
  renderPreview: PreviewRenderer,
  urlApi: UrlApi,
): Promise<PreviewArtifact> {
  const blob = await renderPreview(pixels)
  const url = urlApi.createObjectURL(blob)
  let revoked = false
  return {
    url,
    width: pixels.width,
    height: pixels.height,
    dispose(): void {
      if (!revoked) {
        revoked = true
        urlApi.revokeObjectURL(url)
      }
    },
  }
}

function savedArtifact(name: string, byteLength: number): ExportArtifact {
  return { kind: 'saved', name, byteLength, dispose: () => undefined }
}
