import { describe, expect, it } from 'vitest'
import { countingUrlApi } from '../export/exportTestSupport'
import { exportFilename } from '../export/downloadName'
import { createStudioController } from '../state/controller'
import { ANIMATION_BUDGET_MESSAGE } from './browserAdapter'
import { createNativeProcessingAdapter, NativeUnsupportedInputError } from './nativeAdapter'
import type { NativeBridge, NativeChannel } from './nativeBridge'
import { NATIVE_FRAME_HEADER_BYTES } from './nativeFrame'
import { createNativeFiles, toInput } from './nativeFiles'
import type { NativeDropOutcome } from './nativeFiles'
import type { ExportFormat, JobContext, ProcessingSession, StudioSettings } from './types'
import { DEFAULT_SETTINGS, ExportCancelledError } from './types'
import { ProcessingWorkerError } from './workerClient'
import { DEFAULT_RAMP } from './protocol'

type Handler = (args: Record<string, unknown> | undefined) => unknown

interface Call {
  readonly command: string
  readonly args: Record<string, unknown> | undefined
}

/** Scriptable bridge: each command has a handler, and every call is recorded. */
class FakeBridge implements NativeBridge {
  readonly calls: Call[] = []
  readonly handlers = new Map<string, Handler>()
  readonly channels: Array<NativeChannel<unknown>> = []
  readonly listeners = new Map<string, (payload: unknown) => void>()

  on(command: string, handler: Handler): this {
    this.handlers.set(command, handler)
    return this
  }

  countCalls(command: string): number {
    return this.calls.filter((call) => call.command === command).length
  }

  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    this.calls.push({ command, args })
    const handler = this.handlers.get(command)
    if (handler === undefined) {
      return Promise.reject({ code: 'internal', message: `unhandled ${command}` })
    }
    return new Promise<T>((resolve, reject) => {
      queueMicrotask(() => {
        try {
          resolve(handler(args) as T)
        } catch (error) {
          reject(error)
        }
      })
    })
  }

  channel<T>(): NativeChannel<T> {
    const channel: NativeChannel<T> = { onmessage: () => undefined }
    this.channels.push(channel as NativeChannel<unknown>)
    return channel
  }

  listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
    this.listeners.set(event, handler as (payload: unknown) => void)
    return Promise.resolve(() => {
      this.listeners.delete(event)
    })
  }

  emit(event: string, payload: unknown): void {
    this.listeners.get(event)?.(payload)
  }
}

const session = (): ProcessingSession => ({}) as ProcessingSession
const context = (operationId: number, generation: number): JobContext => ({
  operationId,
  generation,
})

function fakeRender() {
  return async (pixels: { width: number; height: number; pixels: ArrayBuffer }): Promise<Blob> =>
    new Blob([new Uint8Array(pixels.pixels)], { type: `image/png;w=${pixels.width}` })
}

function frameBody(generation: number, width: number, height: number): ArrayBuffer {
  const body = new ArrayBuffer(NATIVE_FRAME_HEADER_BYTES + width * height * 4)
  const bytes = new Uint8Array(body)
  const view = new DataView(body)
  bytes.set(new TextEncoder().encode('I2BF'), 0)
  view.setUint32(4, 1, true)
  view.setBigUint64(8, BigInt(generation), true)
  view.setUint32(16, width, true)
  view.setUint32(20, height, true)
  for (let index = NATIVE_FRAME_HEADER_BYTES; index < bytes.length; index += 1) {
    bytes[index] = 200
  }
  return body
}

function setup(bridge: FakeBridge) {
  const urlApi = countingUrlApi()
  const adapter = createNativeProcessingAdapter({
    bridge,
    renderPreview: fakeRender(),
    urlApi,
  })
  return { adapter, urlApi }
}

async function importDemo(
  adapter: ReturnType<typeof createNativeProcessingAdapter>,
  s: ProcessingSession,
) {
  return adapter.import(s, { kind: 'demo' }, context(1, 1))
}

/** Animation fields of a still import reply: one frame, no delays, loop count 0. */
const STILL_REPLY = { frameCount: 1, frameDelays: [], loopCount: 0 } as const

function demoReply() {
  return { name: 'Demo fixture', width: 256, height: 192, ...STILL_REPLY, source: 'demo' }
}

function rejectionOf(promise: Promise<unknown>): Promise<unknown> {
  return promise.then(
    () => {
      throw new Error('The operation was expected to reject.')
    },
    (error: unknown) => error,
  )
}

describe('native import', () => {
  it('imports the demo through import_demo without sending bytes and reports its metadata', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply)
    const { adapter } = setup(bridge)
    const result = await importDemo(adapter, adapter.createSession())
    expect(bridge.calls).toEqual([{ command: 'import_demo', args: { generation: 1 } }])
    expect(result.metadata).toEqual({
      name: 'Demo fixture',
      width: 256,
      height: 192,
      source: 'demo',
    })
  })

  it('imports a native file by handle and releases the handle afterwards', async () => {
    const bridge = new FakeBridge()
      .on('import_source', () => ({
        name: 'photo.png',
        width: 4,
        height: 3,
        inputBytes: 123,
        ...STILL_REPLY,
        source: 'file',
      }))
      .on('release_handle', () => undefined)
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const input = toInput({ id: 9, name: 'photo.png', byteLength: 123 })
    const result = await adapter.import(s, input, context(1, 1))
    expect(result.metadata).toEqual({
      name: 'photo.png',
      width: 4,
      height: 3,
      source: 'file',
      inputBytes: 123,
    })
    expect(bridge.calls.map((call) => call.command)).toEqual(['import_source', 'release_handle'])
    expect(bridge.calls[0]?.args).toEqual({ fileId: 9, generation: 1 })
    expect(bridge.calls[1]?.args).toEqual({ id: 9 })
  })

  it('releases the handle and surfaces the shell error code and message on a rejected file', async () => {
    const bridge = new FakeBridge()
      .on('import_source', () => {
        throw { code: 'unsupported_format', message: 'not an image' }
      })
      .on('release_handle', () => undefined)
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const error = await rejectionOf(
      adapter.import(s, toInput({ id: 3, name: 'x.txt', byteLength: 4 }), context(1, 1)),
    )
    expect(error).toBeInstanceOf(ProcessingWorkerError)
    expect(error).toMatchObject({ code: 'unsupported_format', message: 'not an image' })
    expect(bridge.countCalls('release_handle')).toBe(1)
  })

  it('rejects browser File inputs, which the native adapter cannot read', async () => {
    const bridge = new FakeBridge()
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const error = await rejectionOf(adapter.import(s, new File(['x'], 'x.png'), context(1, 1)))
    expect(error).toBeInstanceOf(NativeUnsupportedInputError)
    expect(bridge.calls).toEqual([])
  })

  it('makes an earlier source stale once a newer import succeeds', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply).on('reset', () => undefined)
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const first = await importDemo(adapter, s)
    await adapter.import(s, { kind: 'demo' }, context(2, 2))
    await expect(adapter.preview(s, first.source, DEFAULT_SETTINGS, context(3, 3))).rejects.toThrow(
      'does not belong',
    )
  })

  it('resets the shell when the held source is disposed, but not when a replaced source is', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply).on('reset', () => undefined)
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const first = await importDemo(adapter, s)
    const second = await adapter.import(s, { kind: 'demo' }, context(2, 2))
    first.dispose()
    expect(bridge.countCalls('reset')).toBe(0)
    second.dispose()
    expect(bridge.countCalls('reset')).toBe(1)
    second.dispose()
    expect(bridge.countCalls('reset')).toBe(1)
  })
})

describe('native preview', () => {
  const artwork: StudioSettings = {
    cellSize: 12,
    fontSize: 16,
    threshold: 128,
    mode: 'binary' as const,
    ramp: DEFAULT_RAMP,
    foreground: '#00ff00',
    background: '#000000',
    colorMode: 'monochrome',
    contrast: 0,
    invert: false,
  }

  async function withSource(bridge: FakeBridge) {
    const { adapter, urlApi } = setup(bridge)
    const s = adapter.createSession()
    const result = await importDemo(adapter, s)
    return { adapter, urlApi, s, source: result.source }
  }

  it('sends the typed settings and generation, then returns a PNG object URL for the frame', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('start_preview', () => frameBody(5, 2, 1))
    const { adapter, urlApi, s, source } = await withSource(bridge)
    const artifact = await adapter.preview(s, source, artwork, context(2, 5), 'artwork')
    expect(artifact.width).toBe(2)
    expect(artifact.height).toBe(1)
    expect(artifact.url).toBe('blob:export/1')
    expect(urlApi.live.size).toBe(1)
    const call = bridge.calls.find((entry) => entry.command === 'start_preview')
    expect(call?.args).toMatchObject({
      kind: 'artwork',
      generation: 5,
      settings: { cellSize: 12, fontSize: 16, threshold: 128, sampling: 'classic' },
    })
    artifact.dispose()
    expect(urlApi.live.size).toBe(0)
  })

  it('sends the render mode, ramp and mode-forced sampling over IPC', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('start_preview', (args) => frameBody(Number(args?.generation), 2, 1))
    const { adapter, s, source } = await withSource(bridge)
    const ascii: StudioSettings = { ...artwork, mode: 'ascii', ramp: ' .#@' }
    const artifact = await adapter.preview(s, source, ascii, context(2, 5), 'artwork')
    artifact.dispose()
    const block: StudioSettings = { ...artwork, mode: 'block' }
    const second = await adapter.preview(s, source, block, context(3, 6), 'artwork')
    second.dispose()
    const calls = bridge.calls.filter((entry) => entry.command === 'start_preview')
    expect(calls[0]?.args).toMatchObject({
      settings: {
        cellSize: 12,
        fontSize: 16,
        threshold: 128,
        sampling: 'average',
        mode: 'ascii',
        ramp: ' .#@',
      },
    })
    expect(calls[1]?.args).toMatchObject({
      settings: { sampling: 'average', mode: 'block', ramp: DEFAULT_RAMP },
    })
  })

  it('sends the colour style over IPC with the same keys for preview and PNG export', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('start_preview', (args) => frameBody(Number(args?.generation), 2, 1))
      .on('save_image_dialog', () => ({ id: 7, name: 'out.png' }))
      .on('export_file', () => ({ byteLength: 42 }))
    const { adapter, s, source } = await withSource(bridge)
    const styled: StudioSettings = {
      ...artwork,
      foreground: '#112233',
      background: '#f0e0d0',
      colorMode: 'source',
      contrast: -25,
      invert: true,
    }
    const artifact = await adapter.preview(s, source, styled, context(2, 5), 'artwork')
    artifact.dispose()
    await adapter.export(s, source, 'png', styled, context(3, 6))
    const styleKeys = {
      foreground: '#112233',
      background: '#f0e0d0',
      colorMode: 'source',
      contrast: -25,
      invert: true,
    }
    expect(bridge.calls.find((call) => call.command === 'start_preview')?.args).toMatchObject({
      settings: styleKeys,
    })
    expect(bridge.calls.find((call) => call.command === 'export_file')?.args).toMatchObject({
      settings: styleKeys,
    })
  })

  it('requests the original without settings', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('start_preview', () => frameBody(5, 1, 1))
    const { adapter, s, source } = await withSource(bridge)
    await adapter.preview(s, source, artwork, context(2, 5), 'original')
    expect(bridge.calls.find((entry) => entry.command === 'start_preview')?.args).toMatchObject({
      kind: 'original',
      settings: null,
    })
  })

  it('fails typed for a frame that does not match its header, and displays nothing', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply).on('start_preview', () => {
      const body = frameBody(5, 2, 1)
      new Uint8Array(body)[0] = 0x58
      return body
    })
    const { adapter, urlApi, s, source } = await withSource(bridge)
    const error = await rejectionOf(adapter.preview(s, source, artwork, context(2, 5), 'artwork'))
    expect(error).toMatchObject({ code: 'internal' })
    expect((error as Error).message).toMatch(/magic/)
    expect(urlApi.created).toBe(0)
  })

  it('reports stale_generation for a frame from a different generation', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('start_preview', () => frameBody(4, 2, 1))
    const { adapter, urlApi, s, source } = await withSource(bridge)
    const error = await rejectionOf(adapter.preview(s, source, artwork, context(2, 5), 'artwork'))
    expect(error).toMatchObject({ code: 'stale_generation' })
    expect(urlApi.created).toBe(0)
  })

  it('maps progress for its own generation only and clamps it into [0, 1]', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply).on('start_preview', (args) => {
      const channel = args?.progress as NativeChannel<{
        generation: number
        stage: string
        done: number
        total: number
      }>
      channel.onmessage({ generation: 4, stage: 'sampling', done: 9, total: 10 })
      channel.onmessage({ generation: 5, stage: 'sampling', done: 0, total: 10 })
      channel.onmessage({ generation: 5, stage: 'preview', done: 5, total: 10 })
      channel.onmessage({ generation: 5, stage: 'preview', done: 30, total: 10 })
      channel.onmessage({ generation: 5, stage: 'preview', done: 1, total: 0 })
      return frameBody(5, 1, 1)
    })
    const { adapter, s, source } = await withSource(bridge)
    const fractions: number[] = []
    await adapter.preview(s, source, artwork, context(2, 5), 'artwork', (fraction) =>
      fractions.push(fraction),
    )
    expect(fractions).toEqual([0, 0.5, 1])
  })

  it('rejects an unknown preview kind before any call', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply)
    const { adapter, s, source } = await withSource(bridge)
    await expect(
      adapter.preview(s, source, artwork, context(2, 5), 'bogus' as 'artwork'),
    ).rejects.toThrow(RangeError)
    expect(bridge.countCalls('start_preview')).toBe(0)
  })

  it('maps shell rejections to the typed error and a plain error to internal', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply).on('start_preview', () => {
      throw { code: 'no_job', message: 'there is no source; import an image first' }
    })
    const { adapter, s, source } = await withSource(bridge)
    const typed = await rejectionOf(adapter.preview(s, source, artwork, context(2, 5), 'artwork'))
    expect(typed).toMatchObject({
      code: 'no_job',
      message: 'there is no source; import an image first',
    })
    expect(typed).toBeInstanceOf(ProcessingWorkerError)

    const untyped = new FakeBridge().on('import_demo', demoReply).on('start_preview', () => {
      throw new Error('pipe closed')
    })
    const second = await withSource(untyped)
    const plain = await rejectionOf(
      second.adapter.preview(second.s, second.source, artwork, context(2, 5), 'artwork'),
    )
    expect(plain).toMatchObject({ code: 'internal', message: 'pipe closed' })
  })

  it('cancel reaches the shell with the operation generation and suppresses the frame', async () => {
    const gate: { release?: () => void } = {}
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('cancel', () => undefined)
      .on('start_preview', async () => {
        await new Promise<void>((resolve) => {
          gate.release = resolve
        })
        return frameBody(5, 1, 1)
      })
    const { adapter, urlApi, s, source } = await withSource(bridge)
    const pending = rejectionOf(adapter.preview(s, source, artwork, context(2, 5), 'artwork'))
    await waitForCall(bridge, 'start_preview')
    adapter.cancel(s, 2)
    expect(bridge.calls.find((call) => call.command === 'cancel')?.args).toEqual({ generation: 5 })
    gate.release?.()
    expect(await pending).toMatchObject({ code: 'cancelled' })
    expect(urlApi.created).toBe(0)
  })
})

async function waitForCall(bridge: FakeBridge, command: string, count = 1): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (bridge.countCalls(command) >= count) return
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  throw new Error(`${command} was never invoked.`)
}

describe('native export', () => {
  async function ready(bridge: FakeBridge) {
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const result = await adapter.import(s, { kind: 'demo' }, context(1, 1))
    return { adapter, s, source: result.source }
  }

  it('shows the save dialog with the browser file name and exports into the chosen sink', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => ({ id: 7, name: 'out.png' }))
      .on('export_file', (args) => {
        ;(args?.progress as NativeChannel<unknown>).onmessage({
          generation: 4,
          stage: 'encoding',
          done: 1,
          total: 2,
        })
        return { generation: 4, byteLength: 42 }
      })
      .on('release_handle', () => undefined)
    const { adapter, s, source } = await ready(bridge)
    const fractions: number[] = []
    const artifact = await adapter.export(
      s,
      source,
      'png',
      DEFAULT_SETTINGS,
      context(3, 4),
      (fraction) => fractions.push(fraction),
    )
    expect(bridge.calls.find((call) => call.command === 'save_image_dialog')?.args).toEqual({
      suggestedName: exportFilename('Demo fixture'),
      outputKind: 'png',
    })
    expect(bridge.calls.find((call) => call.command === 'export_file')?.args).toMatchObject({
      sinkId: 7,
      generation: 4,
      settings: { cellSize: 12, fontSize: 16, threshold: 128, sampling: 'classic' },
    })
    expect(artifact).toEqual({
      kind: 'saved',
      name: 'out.png',
      byteLength: 42,
      dispose: expect.any(Function),
    })
    expect(fractions).toEqual([0.5])
    expect(bridge.calls.at(-1)).toEqual({ command: 'release_handle', args: { id: 7 } })
  })

  it('sends the render mode and ramp with the native PNG export', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => ({ id: 7, name: 'out.png' }))
      .on('export_file', () => ({ byteLength: 42 }))
    const { adapter, s, source } = await ready(bridge)
    await adapter.export(
      s,
      source,
      'png',
      { ...DEFAULT_SETTINGS, mode: 'ascii', ramp: ' .oO@' },
      context(3, 4),
    )
    expect(bridge.calls.find((call) => call.command === 'export_file')?.args).toMatchObject({
      settings: { sampling: 'average', mode: 'ascii', ramp: ' .oO@' },
    })
  })

  it('treats a dismissed save dialog as ExportCancelledError with no export call', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply).on('save_image_dialog', () => null)
    const { adapter, s, source } = await ready(bridge)
    const error = await rejectionOf(
      adapter.export(s, source, 'png', DEFAULT_SETTINGS, context(3, 4)),
    )
    expect(error).toBeInstanceOf(ExportCancelledError)
    expect(bridge.countCalls('export_file')).toBe(0)
    expect(bridge.countCalls('release_handle')).toBe(0)
  })

  it('releases the sink and surfaces the shell error when the export fails', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => ({ id: 8, name: 'out.png' }))
      .on('export_file', () => {
        throw { code: 'internal', message: 'the file could not be written: PermissionDenied' }
      })
      .on('release_handle', () => undefined)
    const { adapter, s, source } = await ready(bridge)
    const error = await rejectionOf(
      adapter.export(s, source, 'png', DEFAULT_SETTINGS, context(3, 4)),
    )
    expect(error).toMatchObject({
      code: 'internal',
      message: expect.stringContaining('PermissionDenied'),
    })
    expect(bridge.calls.at(-1)).toEqual({ command: 'release_handle', args: { id: 8 } })
  })

  it('does not write when the export was cancelled while the dialog was open', async () => {
    const gate: { answer?: (value: unknown) => void } = {}
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('cancel', () => undefined)
      .on('save_image_dialog', () => new Promise((resolve) => (gate.answer = resolve)))
      .on('release_handle', () => undefined)
    const { adapter, s, source } = await ready(bridge)
    const pending = rejectionOf(adapter.export(s, source, 'png', DEFAULT_SETTINGS, context(3, 4)))
    await waitForCall(bridge, 'save_image_dialog')
    adapter.cancel(s, 3)
    gate.answer?.({ id: 11, name: 'late.png' })
    expect(await pending).toMatchObject({ code: 'cancelled' })
    expect(bridge.countCalls('export_file')).toBe(0)
    expect(bridge.calls.at(-1)).toEqual({ command: 'release_handle', args: { id: 11 } })
  })

  it('exports text through a .txt save dialog and the same file command with its kind', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => ({ id: 7, name: 'grid.txt' }))
      .on('export_file', () => ({ byteLength: 37 }))
    const { adapter, s, source } = await ready(bridge)
    const artifact = await adapter.export(s, source, 'text', DEFAULT_SETTINGS, context(3, 4))
    expect(artifact).toEqual({
      kind: 'saved',
      name: 'grid.txt',
      byteLength: 37,
      dispose: expect.any(Function),
    })
    expect(bridge.calls.find((call) => call.command === 'save_image_dialog')?.args).toEqual({
      suggestedName: exportFilename('Demo fixture', 'text'),
      outputKind: 'text',
    })
    expect(bridge.calls.find((call) => call.command === 'export_file')?.args).toMatchObject({
      outputKind: 'text',
      sinkId: 7,
    })
    expect(bridge.calls.at(-1)).toEqual({ command: 'release_handle', args: { id: 7 } })
  })

  it('rejects an unknown export format before any native command', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply)
    const { adapter, s, source } = await ready(bridge)
    await expect(
      adapter.export(s, source, 'svg' as unknown as ExportFormat, DEFAULT_SETTINGS, context(3, 4)),
    ).rejects.toBeInstanceOf(RangeError)
    expect(bridge.countCalls('save_image_dialog')).toBe(0)
    expect(bridge.countCalls('export_file')).toBe(0)
  })

  it('a dismissed save dialog returns the studio to idle with no error and no failure', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => null)
      .on('reset', () => undefined)
    const adapter = createNativeProcessingAdapter({
      bridge,
      renderPreview: fakeRender(),
      urlApi: countingUrlApi(),
    })
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    await controller.startExport()
    const snapshot = controller.getSnapshot()
    expect(snapshot.export).toEqual({ status: 'idle', progress: 0, byteLength: null, error: null })
    expect(snapshot.status).toBe('ready')
    expect(snapshot.error).toBeNull()
    expect(controller.readyExport()).toBeNull()
  })

  it('a saved export is the ready artifact, and the controller keeps it until reset', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => ({ id: 2, name: 'chosen.png' }))
      .on('export_file', () => ({ generation: 3, byteLength: 9 }))
      .on('release_handle', () => undefined)
      .on('reset', () => undefined)
    const adapter = createNativeProcessingAdapter({
      bridge,
      renderPreview: fakeRender(),
      urlApi: countingUrlApi(),
    })
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    await controller.startExport()
    expect(controller.getSnapshot().export).toMatchObject({ status: 'ready', byteLength: 9 })
    expect(controller.readyExport()).toMatchObject({ kind: 'saved', name: 'chosen.png' })
    controller.reset()
    expect(bridge.countCalls('reset')).toBe(1)
    expect(controller.readyExport()).toBeNull()
  })
})

describe('native cancel and reset', () => {
  it('dispose resets the shell and cancels nothing it no longer owns', async () => {
    const bridge = new FakeBridge().on('import_demo', demoReply).on('reset', () => undefined)
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    await adapter.import(s, { kind: 'demo' }, context(1, 1))
    adapter.dispose(s)
    adapter.dispose(s)
    expect(bridge.countCalls('reset')).toBe(1)
    await expect(adapter.import(s, { kind: 'demo' }, context(2, 2))).rejects.toThrow('disposed')
  })

  it('a session from another adapter is refused', () => {
    const { adapter } = setup(new FakeBridge())
    expect(() => adapter.cancel(session(), 1)).toThrow('does not belong')
  })
})

describe('native file layer', () => {
  it('a cancelled open resolves to null, which the import flow treats as a no-op', async () => {
    const bridge = new FakeBridge().on('open_image_dialog', () => null)
    expect(await createNativeFiles(bridge).openImage()).toBeNull()
  })

  it('an accepted open returns an opaque handle input with name and size', async () => {
    const bridge = new FakeBridge().on('open_image_dialog', () => ({
      id: 4,
      name: 'a.png',
      byteLength: 10,
    }))
    expect(await createNativeFiles(bridge).openImage()).toEqual({
      kind: 'native-file',
      id: 4,
      name: 'a.png',
      byteLength: 10,
    })
  })

  it('a failed open is mapped to the typed error', async () => {
    const bridge = new FakeBridge().on('open_image_dialog', () => {
      throw { code: 'resource_limit_exceeded', message: 'too many handles' }
    })
    await expect(createNativeFiles(bridge).openImage()).rejects.toMatchObject({
      code: 'resource_limit_exceeded',
      message: 'too many handles',
    })
  })

  it('forwards drop outcomes and hover states from the window events', async () => {
    const bridge = new FakeBridge()
    const files = createNativeFiles(bridge)
    const outcomes: NativeDropOutcome[] = []
    const hovers: boolean[] = []
    await files.listenDrops((outcome) => outcomes.push(outcome))
    await files.listenHover((active) => hovers.push(active))
    bridge.emit('native-file-hover', { active: true })
    bridge.emit('native-file-drop', {
      status: 'rejected',
      error: { code: 'invalid_argument', message: 'Choose one image at a time.' },
    })
    bridge.emit('native-file-hover', { active: false })
    expect(hovers).toEqual([true, false])
    expect(outcomes).toEqual([
      {
        status: 'rejected',
        error: { code: 'invalid_argument', message: 'Choose one image at a time.' },
      },
    ])
  })
})

describe('native cancel of an in-flight import', () => {
  it('a cancelled import is never adopted: the held source keeps rendering and no reset is sent', async () => {
    const gate: { finish?: (reply: unknown) => void } = {}
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('import_source', () => new Promise((resolve) => (gate.finish = resolve)))
      .on('cancel', () => undefined)
      .on('reset', () => undefined)
      .on('release_handle', () => undefined)
      .on('start_preview', (args) => frameBody(Number(args?.generation), 2, 1))
    const { adapter } = setup(bridge)
    const controller = createStudioController({ adapter })
    await controller.loadDemo()
    const pending = controller.import(toInput({ id: 9, name: 'b.png', byteLength: 10 }))
    await waitForCall(bridge, 'import_source')
    controller.cancel()
    gate.finish?.({
      name: 'b.png',
      width: 10,
      height: 10,
      inputBytes: 10,
      ...STILL_REPLY,
      source: 'file',
    })
    await pending
    expect(controller.getSnapshot().metadata?.name).toBe('Demo fixture')
    await controller.preview('artwork')
    expect(controller.getSnapshot().status).toBe('ready')
    expect(controller.getSnapshot().preview).not.toBeNull()
    expect(bridge.countCalls('reset')).toBe(0)
    expect(bridge.calls.find((call) => call.command === 'cancel')?.args).toEqual({ generation: 2 })
  })

  it('a preview waits for an earlier cancel to reach the shell before it starts', async () => {
    const gates: { previews: Array<() => void>; cancel?: () => void } = { previews: [] }
    const order: string[] = []
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('cancel', () => {
        order.push('cancel')
        return new Promise<void>((resolve) => {
          gates.cancel = resolve
        })
      })
      .on('start_preview', (args) => {
        order.push('preview')
        return new Promise((resolve) => {
          gates.previews.push(() => resolve(frameBody(Number(args?.generation), 2, 1)))
        })
      })
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const result = await importDemo(adapter, s)
    const first = adapter.preview(s, result.source, DEFAULT_SETTINGS, context(7, 5), 'artwork')
    await waitForCall(bridge, 'start_preview')
    adapter.cancel(s, 7)
    await waitForCall(bridge, 'cancel')
    const later = adapter.preview(s, result.source, DEFAULT_SETTINGS, context(8, 6), 'artwork')
    await new Promise((resolve) => setTimeout(resolve, 5))
    expect(order).toEqual(['preview', 'cancel'])
    gates.cancel?.()
    await waitForCall(bridge, 'start_preview', 2)
    for (const release of gates.previews) release()
    await first.catch(() => undefined)
    await later
    expect(order).toEqual(['preview', 'cancel', 'preview'])
  })
})

describe('native reply validation', () => {
  it('a malformed import reply is an internal error with a readable message', async () => {
    const bridge = new FakeBridge().on('import_demo', () => ({ name: 'x', width: -1, height: 1 }))
    const { adapter } = setup(bridge)
    const error = await rejectionOf(
      adapter.import(adapter.createSession(), { kind: 'demo' }, context(1, 1)),
    )
    expect(error).toMatchObject({
      code: 'internal',
      message: 'The native shell sent a malformed reply.',
    })
  })

  it('a malformed save reply is refused before any export', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => ({ id: -4, name: 'x.png' }))
      .on('release_handle', () => undefined)
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const source = (await importDemo(adapter, s)).source
    const error = await rejectionOf(
      adapter.export(s, source, 'png', DEFAULT_SETTINGS, context(3, 4)),
    )
    expect(error).toMatchObject({ code: 'internal' })
    expect(bridge.countCalls('export_file')).toBe(0)
  })

  it('a malformed export byte length is refused, not shown as a saved size', async () => {
    const bridge = new FakeBridge()
      .on('import_demo', demoReply)
      .on('save_image_dialog', () => ({ id: 6, name: 'x.png' }))
      .on('export_file', () => ({ generation: 4, byteLength: 1.5 }))
      .on('release_handle', () => undefined)
    const { adapter } = setup(bridge)
    const s = adapter.createSession()
    const source = (await importDemo(adapter, s)).source
    const error = await rejectionOf(
      adapter.export(s, source, 'png', DEFAULT_SETTINGS, context(3, 4)),
    )
    expect(error).toMatchObject({
      code: 'internal',
      message: 'The native shell sent a malformed reply.',
    })
    expect(bridge.calls.at(-1)).toEqual({ command: 'release_handle', args: { id: 6 } })
  })

  it('a rejection with a code but no message reports the fixed fallback text', async () => {
    const bridge = new FakeBridge().on('import_demo', () => {
      throw { code: 'decode_failed' }
    })
    const { adapter } = setup(bridge)
    const error = await rejectionOf(
      adapter.import(adapter.createSession(), { kind: 'demo' }, context(1, 1)),
    )
    expect(error).toMatchObject({
      code: 'decode_failed',
      message: 'The native shell could not complete the request.',
    })
  })
})

/** Import reply of an animated file; `loopCount` `null` is an endless loop. */
function animatedReply(frameCount: number, frameDelays: number[], loopCount: number | null) {
  return {
    name: 'spin.gif',
    width: 1,
    height: 1,
    inputBytes: 40,
    frameCount,
    frameDelays,
    loopCount,
    source: 'file',
  }
}

/** A one-pixel frame whose pixel carries the frame index, so tests can tell frames apart. */
function markedFrameBody(generation: number, frame: number): ArrayBuffer {
  const body = frameBody(generation, 1, 1)
  new Uint8Array(body)[NATIVE_FRAME_HEADER_BYTES] = frame
  return body
}

function markingRender() {
  return async (pixels: { pixels: ArrayBuffer }): Promise<Blob> =>
    new Blob([], { type: `image/png;frame=${new Uint8Array(pixels.pixels)[0] ?? -1}` })
}

/** URL api that records each object's type under its URL and tracks which URLs are live. */
function recordingUrlApi() {
  const types = new Map<string, string>()
  const live = new Set<string>()
  let created = 0
  return {
    types,
    live,
    get created(): number {
      return created
    },
    api: {
      createObjectURL(blob: Blob): string {
        created += 1
        const url = `blob:frames/${created}`
        types.set(url, blob.type)
        live.add(url)
        return url
      },
      revokeObjectURL(url: string): void {
        live.delete(url)
      },
    },
  }
}

describe('native animation', () => {
  async function animatedSource(bridge: FakeBridge, frameCount: number) {
    bridge
      .on('import_source', () => animatedReply(frameCount, [5, 5, 5].slice(0, frameCount), 0))
      .on('release_handle', () => undefined)
    const recording = recordingUrlApi()
    const adapter = createNativeProcessingAdapter({
      bridge,
      renderPreview: markingRender(),
      urlApi: recording.api,
    })
    const s = adapter.createSession()
    const result = await adapter.import(
      s,
      toInput({ id: 9, name: 'spin.gif', byteLength: 40 }),
      context(1, 1),
    )
    return { adapter, s, source: result.source, result, recording }
  }

  it('adopts the frame timing and loop count of an animated import, and none for a still', async () => {
    const bridge = new FakeBridge()
      .on('import_source', () => animatedReply(3, [5, 20, 7], null))
      .on('import_demo', demoReply)
      .on('release_handle', () => undefined)
    const { adapter } = setup(bridge)
    const animated = await adapter.import(
      adapter.createSession(),
      toInput({ id: 9, name: 'spin.gif', byteLength: 40 }),
      context(1, 1),
    )
    expect(animated.animation).toEqual({ frameCount: 3, delaysCs: [5, 20, 7], loopCount: Infinity })
    const still = await importDemo(adapter, adapter.createSession())
    expect(still.animation).toBeNull()
  })

  it('refuses an animation reply whose delays do not match its frame count', async () => {
    const bridge = new FakeBridge()
      .on('import_source', () => animatedReply(3, [5, 20], 0))
      .on('release_handle', () => undefined)
    const { adapter } = setup(bridge)
    const error = await rejectionOf(
      adapter.import(
        adapter.createSession(),
        toInput({ id: 9, name: 'spin.gif', byteLength: 40 }),
        context(1, 1),
      ),
    )
    expect(error).toMatchObject({ code: 'internal' })
  })

  it('renders every frame from the start frame with wrap-around, in frame order, reporting progress', async () => {
    const bridge = new FakeBridge().on('start_preview', (args) =>
      markedFrameBody(5, Number(args?.frame)),
    )
    const { adapter, s, source, recording } = await animatedSource(bridge, 3)
    const fractions: number[] = []
    const artifact = await adapter.preview(
      s,
      source,
      DEFAULT_SETTINGS,
      context(2, 5),
      'artwork',
      (fraction) => fractions.push(fraction),
      { startFrame: 1 },
    )
    const frames = bridge.calls
      .filter((call) => call.command === 'start_preview')
      .map((call) => call.args?.frame)
    expect(frames).toEqual([1, 2, 0])
    expect(artifact.frames?.map((url) => recording.types.get(url))).toEqual([
      'image/png;frame=0',
      'image/png;frame=1',
      'image/png;frame=2',
    ])
    expect(artifact.url).toBe(artifact.frames?.[1])
    expect(fractions).toEqual([1 / 3, 2 / 3, 1])
    artifact.dispose()
    expect(recording.live.size).toBe(0)
  })

  it('a cancel between frames stops the run, starts no later frame and revokes the frames made', async () => {
    const gate: { release?: () => void } = {}
    const bridge = new FakeBridge()
      .on('cancel', () => undefined)
      .on('start_preview', async (args) => {
        if (args?.frame === 1) {
          await new Promise<void>((resolve) => {
            gate.release = resolve
          })
        }
        return markedFrameBody(5, Number(args?.frame))
      })
    const { adapter, s, source, recording } = await animatedSource(bridge, 3)
    const pending = rejectionOf(
      adapter.preview(s, source, DEFAULT_SETTINGS, context(2, 5), 'artwork', undefined, {
        startFrame: 0,
      }),
    )
    await waitForCall(bridge, 'start_preview', 2)
    adapter.cancel(s, 2)
    gate.release?.()
    expect(await pending).toMatchObject({ code: 'cancelled' })
    expect(bridge.countCalls('start_preview')).toBe(2)
    expect(bridge.calls.find((call) => call.command === 'cancel')?.args).toEqual({ generation: 5 })
    expect(recording.created).toBe(1)
    expect(recording.live.size).toBe(0)
  })

  it('a failing frame revokes the frames already made and surfaces the shell error', async () => {
    const bridge = new FakeBridge().on('start_preview', (args) => {
      if (args?.frame === 2) throw { code: 'internal', message: 'boom' }
      return markedFrameBody(5, Number(args?.frame))
    })
    const { adapter, s, source, recording } = await animatedSource(bridge, 3)
    const error = await rejectionOf(
      adapter.preview(s, source, DEFAULT_SETTINGS, context(2, 5), 'artwork', undefined, {
        startFrame: 0,
      }),
    )
    expect(error).toMatchObject({ code: 'internal', message: 'boom' })
    expect(recording.created).toBe(2)
    expect(recording.live.size).toBe(0)
  })

  it('refuses a run whose encoded frames exceed the 512 MiB budget and revokes the frames made', async () => {
    const bridge = new FakeBridge().on('start_preview', (args) =>
      markedFrameBody(5, Number(args?.frame)),
    )
    const recording = recordingUrlApi()
    const adapter = createNativeProcessingAdapter({
      bridge: bridge
        .on('import_source', () => animatedReply(3, [5, 5, 5], 0))
        .on('release_handle', () => undefined),
      renderPreview: async () =>
        ({ size: 300 * 1024 * 1024, type: 'image/png' }) as unknown as Blob,
      urlApi: recording.api,
    })
    const s = adapter.createSession()
    const { source } = await adapter.import(
      s,
      toInput({ id: 9, name: 'spin.gif', byteLength: 40 }),
      context(1, 1),
    )
    const error = await rejectionOf(
      adapter.preview(s, source, DEFAULT_SETTINGS, context(2, 5), 'artwork', undefined, {
        startFrame: 0,
      }),
    )
    expect(error).toEqual(new RangeError(ANIMATION_BUDGET_MESSAGE))
    expect(recording.live.size).toBe(0)
  })

  it('refuses a start frame outside the source before any shell call', async () => {
    const bridge = new FakeBridge()
    const { adapter, s, source } = await animatedSource(bridge, 3)
    const error = await rejectionOf(
      adapter.preview(s, source, DEFAULT_SETTINGS, context(2, 5), 'artwork', undefined, {
        startFrame: 3,
      }),
    )
    expect(error).toBeInstanceOf(RangeError)
    expect(bridge.countCalls('start_preview')).toBe(0)
  })

  it('export sends the frame index and refuses a frame outside the source before the save dialog', async () => {
    const bridge = new FakeBridge()
      .on('save_image_dialog', () => ({ id: 7, name: 'spin.png' }))
      .on('export_file', () => ({ byteLength: 4 }))
      .on('release_handle', () => undefined)
    const { adapter, s, source } = await animatedSource(bridge, 3)
    const artifact = await adapter.export(
      s,
      source,
      'png',
      DEFAULT_SETTINGS,
      context(3, 6),
      undefined,
      {
        frame: 2,
      },
    )
    expect(artifact).toMatchObject({ kind: 'saved', byteLength: 4 })
    expect(bridge.calls.find((call) => call.command === 'export_file')?.args).toMatchObject({
      frame: 2,
      generation: 6,
    })
    const error = await rejectionOf(
      adapter.export(s, source, 'png', DEFAULT_SETTINGS, context(4, 7), undefined, { frame: 3 }),
    )
    expect(error).toBeInstanceOf(RangeError)
    expect(bridge.countCalls('save_image_dialog')).toBe(1)
  })
})

describe('native GIF export', () => {
  async function animatedExport(bridge: FakeBridge, frameCount: number) {
    bridge
      .on('import_source', () =>
        animatedReply(frameCount, frameCount > 1 ? [5, 5, 5].slice(0, frameCount) : [], 0),
      )
      .on('release_handle', () => undefined)
      .on('save_image_dialog', () => ({ id: 7, name: 'spin-binary.gif' }))
      .on('export_file', () => ({ generation: 4, byteLength: 42 }))
    const adapter = createNativeProcessingAdapter({
      bridge,
      renderPreview: markingRender(),
      urlApi: recordingUrlApi().api,
    })
    const s = adapter.createSession()
    const result = await adapter.import(
      s,
      toInput({ id: 9, name: 'spin.gif', byteLength: 40 }),
      context(1, 1),
    )
    return { adapter, s, source: result.source }
  }

  it('passes output gif with no frame and suggests a .gif name', async () => {
    const bridge = new FakeBridge()
    const { adapter, s, source } = await animatedExport(bridge, 3)
    const artifact = await adapter.export(s, source, 'gif', DEFAULT_SETTINGS, context(3, 4))
    expect(bridge.calls.find((call) => call.command === 'save_image_dialog')?.args).toEqual({
      suggestedName: 'spin-binary.gif',
      outputKind: 'gif',
    })
    const args = bridge.calls.find((call) => call.command === 'export_file')?.args
    expect(args).toMatchObject({ outputKind: 'gif', sinkId: 7, generation: 4 })
    expect(args).not.toHaveProperty('frame')
    expect(artifact).toMatchObject({ kind: 'saved', name: 'spin-binary.gif' })
  })

  it('refuses a GIF export of a still before the save dialog opens', async () => {
    const bridge = new FakeBridge()
    const { adapter, s, source } = await animatedExport(bridge, 1)
    await expect(adapter.export(s, source, 'gif', DEFAULT_SETTINGS, context(3, 4))).rejects.toThrow(
      RangeError,
    )
    expect(bridge.countCalls('save_image_dialog')).toBe(0)
  })
})
