import { describe, expect, it } from 'vitest'
import { startHeifDecode } from './heifDecodeJob'
import { MAX_SOURCE_PIXELS } from './protocol'
import { ProcessingWorkerError } from './workerClient'
import { FakeHeifEndpoint, fakeHeifFactory, settle } from './testKit'

const file = new File([new Uint8Array([1, 2, 3, 4])], 'photo.heic', { type: 'image/heic' })

function decodedReply(width = 2, height = 1): unknown {
  return {
    type: 'decoded',
    width,
    height,
    pixels: new Uint8Array(width * height * 4).fill(9).buffer,
  }
}

async function startAndReply(reply: unknown) {
  const factory = fakeHeifFactory()
  const job = startHeifDecode(file, factory.create)
  const endpoint = factory.endpoints[0] as FakeHeifEndpoint
  endpoint.reply(reply)
  return { job, endpoint, factory }
}

describe('startHeifDecode', () => {
  it('posts the file and the shared pixel ceiling, then resolves with the transferred pixels', async () => {
    const factory = fakeHeifFactory()
    const job = startHeifDecode(file, factory.create)
    const endpoint = factory.endpoints[0] as FakeHeifEndpoint
    expect(endpoint.requests).toHaveLength(1)
    const request = endpoint.requests[0] as { type: string; file: File; maxPixels: number }
    expect(request.type).toBe('decode')
    expect(request.file).toBe(file)
    expect(request.maxPixels).toBe(MAX_SOURCE_PIXELS)

    const reply = decodedReply()
    endpoint.reply(reply)
    const decoded = await job.result
    expect(decoded.width).toBe(2)
    expect(decoded.height).toBe(1)
    expect(decoded.pixels).toBe((reply as { pixels: ArrayBuffer }).pixels)
  })

  it('terminates the worker exactly once after success and ignores later events', async () => {
    const { job, endpoint } = await startAndReply(decodedReply())
    await job.result
    expect(endpoint.terminations).toBe(1)
    endpoint.reply(decodedReply())
    endpoint.crash()
    job.cancel()
    expect(endpoint.terminations).toBe(1)
  })

  it('terminates exactly once on cancel and rejects with a cancelled error', async () => {
    const factory = fakeHeifFactory()
    const job = startHeifDecode(file, factory.create)
    const endpoint = factory.endpoints[0] as FakeHeifEndpoint
    const pending = settle(job.result)
    job.cancel()
    job.cancel()
    const outcome = await pending
    expect(outcome.ok).toBe(false)
    expect(outcome.ok ? null : (outcome.error as ProcessingWorkerError).code).toBe('cancelled')
    expect(endpoint.terminations).toBe(1)

    endpoint.reply(decodedReply())
    expect(endpoint.terminations).toBe(1)
  })

  it.each([
    [1, 'unsupported_format', /does not support/],
    [2, 'decode_failed', /damaged or incomplete/],
    [3, 'resource_limit_exceeded', /size limits/],
    [4, 'resource_limit_exceeded', /ran out of memory/],
    [0, 'decoder_failed', /could not run/],
    [17, 'decoder_failed', /could not run/],
  ])('maps status %i to %s with actionable copy', async (status, code, text) => {
    const { job, endpoint } = await startAndReply({ type: 'failed', status })
    const outcome = await settle(job.result)
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    const error = outcome.error as ProcessingWorkerError
    expect(error).toBeInstanceOf(ProcessingWorkerError)
    expect(error.code).toBe(code)
    expect(error.message).toMatch(text)
    expect(endpoint.terminations).toBe(1)
  })

  it('maps an unavailable build to codec_unavailable', async () => {
    const { job, endpoint } = await startAndReply({ type: 'unavailable' })
    const outcome = await settle(job.result)
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe('codec_unavailable')
    expect(outcome.error.message).toBe(
      'HEIC and AVIF are not available in this build. Export the image as JPEG, PNG or WebP and try again.',
    )
    expect(endpoint.terminations).toBe(1)
  })

  it.each([
    ['a worker error event', (endpoint: FakeHeifEndpoint) => endpoint.crash()],
    ['a messageerror event', (endpoint: FakeHeifEndpoint) => endpoint.deliver('messageerror', {})],
    ['a malformed reply', (endpoint: FakeHeifEndpoint) => endpoint.reply({ type: 'bogus' })],
    [
      'a decoded reply with a byte length that disagrees with its size',
      (endpoint: FakeHeifEndpoint) =>
        endpoint.reply({ type: 'decoded', width: 2, height: 1, pixels: new ArrayBuffer(3) }),
    ],
  ])('fails as decoder_failed on %s and terminates once', async (_label, trigger) => {
    const factory = fakeHeifFactory()
    const job = startHeifDecode(file, factory.create)
    const endpoint = factory.endpoints[0] as FakeHeifEndpoint
    trigger(endpoint)
    const outcome = await settle(job.result)
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe('decoder_failed')
    expect(endpoint.terminations).toBe(1)
  })

  it('fails as decoder_failed when the worker cannot be created', async () => {
    const job = startHeifDecode(file, () => {
      throw new Error('Worker constructor refused')
    })
    const outcome = await settle(job.result)
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe('decoder_failed')
  })

  it('terminates and fails as decoder_failed when posting the request throws', async () => {
    const factory = fakeHeifFactory()
    const endpoint = factory.create()
    endpoint.postError = new Error('DataCloneError')
    const job = startHeifDecode(file, () => endpoint)
    const outcome = await settle(job.result)
    expect(outcome.ok).toBe(false)
    if (outcome.ok) return
    expect((outcome.error as ProcessingWorkerError).code).toBe('decoder_failed')
    expect(endpoint.terminations).toBe(1)
  })
})
