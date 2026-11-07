import type { OutputKindName } from './protocol'
import { describe, expect, it } from 'vitest'
import { createBrowserProcessingAdapter } from './browserAdapter'
import { createStudioController } from '../state/controller'
import type { StudioController } from '../state/controller'
import { MAX_SOURCE_PIXELS } from './protocol'
import type { PreviewParameters } from './protocol'
import type { JobContext, ProcessingAdapter, ProcessingSession } from './types'
import { ProcessingWorkerError } from './workerClient'
import type {
  EncodedImportSource,
  ExportCompletion,
  ExportSink,
  PreviewPixels,
  ProcessingClient,
  Releasable,
  RgbaImport,
  RgbaImportSource,
  SourceHandle,
} from './workerClient'
import { fakeHeifFactory, settle, until } from './testKit'
import type { FakeHeifEndpoint, FakeHeifFactory } from './testKit'

const context = (operationId: number): JobContext => ({ operationId, generation: operationId })

const heic = new File([new Uint8Array([0, 0, 0, 24, 102, 116, 121, 112])], 'photo.heic', {
  type: 'image/heic',
})
const png = new File([new Uint8Array([137, 80, 78, 71])], 'plain.png', { type: 'image/png' })

const codecUnavailable = () =>
  new ProcessingWorkerError('codec_unavailable', 'heic decoding is not available in this build')

function decodedRgba(
  width = 2,
  height = 1,
): { type: 'decoded'; width: number; height: number; pixels: ArrayBuffer } {
  return {
    type: 'decoded',
    width,
    height,
    pixels: new Uint8Array(width * height * 4).fill(200).buffer,
  }
}

class FakeClient implements ProcessingClient {
  encodedError: Error | null = null
  importEncodedCalls = 0
  importedCalls = 0
  imported: RgbaImport | null = null
  cancelled: number[] = []
  clientDisposals = 0

  private handle(name: string, width: number, height: number): SourceHandle {
    return { sourceId: 1, name, width, height, dispose: () => undefined }
  }

  async importEncoded(source: EncodedImportSource): Promise<SourceHandle> {
    this.importEncodedCalls += 1
    if (this.encodedError !== null) throw this.encodedError
    const image = typeof source === 'function' ? await source() : source
    return this.handle(image.name, 3, 2)
  }

  async importRgba(source: RgbaImportSource): Promise<SourceHandle> {
    this.importedCalls += 1
    const image = typeof source === 'function' ? await source() : source
    this.imported = image
    return this.handle(image.name, image.width, image.height)
  }

  preview<T extends Releasable>(): Promise<T> {
    return Promise.reject(new Error('preview is not faked in this suite'))
  }

  previewPixels(): Promise<PreviewPixels> {
    return Promise.reject(new Error('preview is not faked in this suite'))
  }

  originalPreview<T extends Releasable>(): Promise<T> {
    return Promise.reject(new Error('preview is not faked in this suite'))
  }

  originalPreviewPixels(): Promise<PreviewPixels> {
    return Promise.reject(new Error('preview is not faked in this suite'))
  }

  exportFile<T extends Releasable>(
    _source: SourceHandle,
    _parameters: PreviewParameters,
    _outputKind: OutputKindName,
    _operationId: number,
    _sink: ExportSink,
    _finalize: (completion: ExportCompletion) => T,
  ): Promise<T> {
    return Promise.reject(new Error('export is not faked in this suite'))
  }

  cancel(operationId: number): void {
    this.cancelled.push(operationId)
  }

  dispose(): void {
    this.clientDisposals += 1
  }
}

interface Harness {
  readonly adapter: ProcessingAdapter
  readonly session: ProcessingSession
  readonly client: FakeClient
  readonly heif: FakeHeifFactory
}

function harness(heif: FakeHeifFactory = fakeHeifFactory()): Harness {
  const client = new FakeClient()
  const adapter = createBrowserProcessingAdapter({
    createClient: () => client,
    createHeifDecoderEndpoint: heif.create,
  })
  return { adapter, session: adapter.createSession(), client, heif }
}

function controllerFor(h: Harness): StudioController {
  return createStudioController({ adapter: h.adapter })
}

async function endpointOf(heif: FakeHeifFactory): Promise<FakeHeifEndpoint> {
  await until(() => heif.endpoints.length > 0, 'a decoder worker')
  const endpoint = heif.endpoints[0]
  if (endpoint === undefined) throw new Error('decoder worker missing')
  return endpoint
}

describe('lazy HEIC/AVIF fallback in the browser adapter', () => {
  it('never creates a decoder worker for an ordinary format', async () => {
    const h = harness()
    const result = await h.adapter.import(h.session, png, context(1))
    expect(result.metadata.source).toBe('file')
    expect(h.client.importEncodedCalls).toBe(1)
    expect(h.client.importedCalls).toBe(0)
    expect(h.heif.endpoints).toHaveLength(0)
  })

  it('decodes codec_unavailable input in a fresh worker and hands RGBA to the processing client', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const pending = h.adapter.import(h.session, heic, context(1))
    const endpoint = await endpointOf(h.heif)

    const request = endpoint.requests[0] as { type: string; file: File; maxPixels: number }
    expect(request.type).toBe('decode')
    expect(request.file).toBe(heic)
    expect(request.maxPixels).toBe(MAX_SOURCE_PIXELS)

    const reply = decodedRgba()
    endpoint.reply(reply)
    const result = await pending

    expect(h.client.imported?.pixels).toBe(reply.pixels)
    expect(h.client.imported?.width).toBe(2)
    expect(h.client.imported?.name).toBe('photo.heic')
    expect(result.metadata).toEqual({
      name: 'photo.heic',
      width: 2,
      height: 1,
      source: 'file',
      inputBytes: heic.size,
    })
    expect(endpoint.terminations).toBe(1)
    expect(h.heif.endpoints).toHaveLength(1)
  })

  it.each([
    [1, 'unsupported_format', /does not support/],
    [2, 'decode_failed', /damaged or incomplete/],
    [3, 'resource_limit_exceeded', /size limits/],
    [4, 'resource_limit_exceeded', /ran out of memory/],
  ])('maps decoder status %i to %s and terminates the worker', async (status, code, text) => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const pending = settle(h.adapter.import(h.session, heic, context(1)))
    const endpoint = await endpointOf(h.heif)
    endpoint.reply({ type: 'failed', status })
    const outcome = await pending
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe(code)
    expect(outcome.error.message).toMatch(text)
    expect(h.client.importedCalls).toBe(0)
    expect(endpoint.terminations).toBe(1)
  })

  it('reports an unavailable build as a typed error through the controller', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const controller = controllerFor(h)
    const pending = settle(controller.import(heic))
    const endpoint = await endpointOf(h.heif)
    endpoint.reply({ type: 'unavailable' })
    const outcome = await pending
    expect(outcome.ok).toBe(false)
    expect(controller.getSnapshot().status).toBe('error')
    expect(controller.getSnapshot().error).toBe(
      'HEIC and AVIF are not available in this build. Export the image as JPEG, PNG or WebP and try again.',
    )
    expect(endpoint.terminations).toBe(1)
    expect(h.client.importedCalls).toBe(0)
  })

  it.each([
    ['a worker crash', (endpoint: FakeHeifEndpoint) => endpoint.crash()],
    ['a messageerror event', (endpoint: FakeHeifEndpoint) => endpoint.deliver('messageerror', {})],
    [
      'a load failure',
      (endpoint: FakeHeifEndpoint) => endpoint.reply({ type: 'failed', status: 0 }),
    ],
  ])('turns %s into decoder_failed without hanging', async (_label, trigger) => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const pending = settle(h.adapter.import(h.session, heic, context(1)))
    const endpoint = await endpointOf(h.heif)
    trigger(endpoint)
    const outcome = await pending
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe('decoder_failed')
    expect(endpoint.terminations).toBe(1)
  })

  it('fails as decoder_failed when the worker cannot be created', async () => {
    const client = new FakeClient()
    client.encodedError = codecUnavailable()
    const adapter = createBrowserProcessingAdapter({
      createClient: () => client,
      createHeifDecoderEndpoint: () => {
        throw new Error('Worker constructor refused')
      },
    })
    const session = adapter.createSession()
    const outcome = await settle(adapter.import(session, heic, context(1)))
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe('decoder_failed')
  })

  it('propagates codec_unavailable unchanged when no decoder factory is configured', async () => {
    const client = new FakeClient()
    client.encodedError = codecUnavailable()
    const adapter = createBrowserProcessingAdapter({ createClient: () => client })
    const session = adapter.createSession()
    const outcome = await settle(adapter.import(session, heic, context(1)))
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe('codec_unavailable')
  })

  it.each([
    ['decode_failed', new ProcessingWorkerError('decode_failed', 'damaged input')],
    ['unsupported_format', new ProcessingWorkerError('unsupported_format', 'unsupported input')],
  ])(
    'propagates a %s import error unchanged and never creates a decoder worker',
    async (_code, error) => {
      const h = harness()
      h.client.encodedError = error
      const outcome = await settle(h.adapter.import(h.session, heic, context(1)))
      expect(outcome.ok).toBe(false)
      if (outcome.ok) return
      expect(outcome.error).toBe(error)
      expect(h.heif.endpoints).toHaveLength(0)
      expect(h.client.importedCalls).toBe(0)
    },
  )

  it('cancelling a decode terminates the worker, and a later reply changes nothing', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const pending = settle(h.adapter.import(h.session, heic, context(1)))
    const endpoint = await endpointOf(h.heif)
    h.adapter.cancel(h.session, 1)
    const outcome = await pending
    expect(outcome.ok).toBe(false)
    expect(endpoint.terminations).toBe(1)
    endpoint.reply(decodedRgba())
    expect(h.client.importedCalls).toBe(0)
    expect(endpoint.terminations).toBe(1)
  })

  it('disposing the session during a decode terminates the worker', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const pending = settle(h.adapter.import(h.session, heic, context(1)))
    const endpoint = await endpointOf(h.heif)
    h.adapter.dispose(h.session)
    const outcome = await pending
    expect(outcome.ok).toBe(false)
    expect(endpoint.terminations).toBe(1)
    endpoint.reply(decodedRgba())
    expect(h.client.importedCalls).toBe(0)
  })
})

describe('controller lifecycle around the decoder worker', () => {
  it('reset during a decode terminates the worker and leaves no source', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const controller = controllerFor(h)
    const pending = settle(controller.import(heic))
    const endpoint = await endpointOf(h.heif)
    controller.reset()
    expect(endpoint.terminations).toBe(1)
    endpoint.reply(decodedRgba())
    await pending
    expect(controller.getSnapshot().metadata).toBeNull()
    expect(controller.getSnapshot().status).toBe('idle')
    expect(h.client.importedCalls).toBe(0)
  })

  it('a newer import terminates the decoder worker and its late reply is ignored', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const controller = controllerFor(h)
    const first = settle(controller.import(heic))
    const endpoint = await endpointOf(h.heif)
    h.client.encodedError = null
    await controller.import(png)
    expect(endpoint.terminations).toBe(1)
    endpoint.reply(decodedRgba())
    await first
    expect(controller.getSnapshot().metadata?.name).toBe('plain.png')
    expect(h.client.importedCalls).toBe(0)
  })

  it('a late decoded result never replaces the current source', async () => {
    const h = harness()
    const controller = controllerFor(h)
    await controller.import(png)
    h.client.encodedError = codecUnavailable()
    const pending = settle(controller.import(heic))
    const endpoint = await endpointOf(h.heif)

    endpoint.reply(decodedRgba())
    h.client.encodedError = null
    await controller.import(new File([new Uint8Array([1])], 'next.png', { type: 'image/png' }))
    await pending

    expect(controller.getSnapshot().metadata?.name).toBe('next.png')
    expect(h.client.importedCalls).toBe(0)
    expect(endpoint.terminations).toBe(1)
  })

  it('disposing the controller during a decode terminates the worker', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const controller = controllerFor(h)
    const pending = settle(controller.import(heic))
    const endpoint = await endpointOf(h.heif)
    controller.dispose()
    await pending
    expect(endpoint.terminations).toBe(1)
    endpoint.reply(decodedRgba())
    expect(controller.getSnapshot().metadata).toBeNull()
    expect(h.client.importedCalls).toBe(0)
  })

  it('clears the loading stage when the decoder reports a failure', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const controller = controllerFor(h)
    const pending = settle(controller.import(heic))
    const endpoint = await endpointOf(h.heif)
    expect(controller.getSnapshot().importStage).toBe('heifDecoder')
    endpoint.reply({ type: 'failed', status: 2 })
    await pending
    expect(controller.getSnapshot().status).toBe('error')
    expect(controller.getSnapshot().importStage).toBeNull()
  })

  it('exposes the loading stage only while the decoder runs', async () => {
    const h = harness()
    h.client.encodedError = codecUnavailable()
    const controller = controllerFor(h)
    const stages: Array<string | null> = []
    controller.subscribe(() => stages.push(controller.getSnapshot().importStage))
    const pending = controller.import(heic)
    const endpoint = await endpointOf(h.heif)
    expect(controller.getSnapshot().importStage).toBe('heifDecoder')
    endpoint.reply(decodedRgba())
    await pending
    expect(controller.getSnapshot().importStage).toBeNull()
    expect(stages).toContain('heifDecoder')
  })
})
