import { HEIF_CODEC_UNAVAILABLE } from './heifDecodeProtocol'
import type { HeifDecodeReply, HeifDecodeRequest } from './heifDecodeProtocol'
import { validateEncodedImport } from './protocol'

/** The subset of the Emscripten decoder module used by `heif_decode.c`. */
export interface HeifModule {
  readonly HEAPU8: Uint8Array<ArrayBuffer>
  _malloc(size: number): number
  _free(pointer: number): void
  _hwc_decode(pointer: number, size: number, maxPixels: number): number
  _hwc_width(): number
  _hwc_height(): number
  _hwc_pixels(): number
  _hwc_release(): void
}

const STATUS_OK = 0
const STATUS_INVALID = 2
const STATUS_LIMIT = 3
const STATUS_NO_MEMORY = 4

/**
 * Decodes one encoded image with `module` and copies the RGBA result into a fresh buffer that the
 * caller owns. The decoder result and the input allocation are released on every path, so the
 * returned `pixels` never alias the WASM heap.
 */
export function decodeWithModule(
  module: HeifModule,
  bytes: Uint8Array<ArrayBuffer>,
  maxPixels: number,
): HeifDecodeReply {
  const pointer = module._malloc(Math.max(bytes.byteLength, 1))
  if (pointer === 0) return { type: 'failed', status: STATUS_NO_MEMORY }
  try {
    module.HEAPU8.set(bytes, pointer)
    const status = module._hwc_decode(pointer, bytes.byteLength, maxPixels)
    if (status !== STATUS_OK) return { type: 'failed', status }
    const width = module._hwc_width()
    const height = module._hwc_height()
    const source = module._hwc_pixels()
    const pixels = module.HEAPU8.slice(source, source + width * height * 4).buffer
    return { type: 'decoded', width, height, pixels }
  } finally {
    module._hwc_release()
    module._free(pointer)
  }
}

/**
 * Worker-side decode of one request. An encoded size outside the shared bounds is refused before
 * the decoder loads. Load and run failures become `failed` with status 0, and a decoder build
 * without codecs becomes `unavailable`. Never rejects.
 */
export async function runHeifDecode(
  loadModule: () => Promise<HeifModule>,
  request: HeifDecodeRequest,
): Promise<HeifDecodeReply> {
  const invalid = validateEncodedImport(request.file.name, request.file.size)
  if (invalid !== null) {
    return {
      type: 'failed',
      status: invalid.code === 'resource_limit_exceeded' ? STATUS_LIMIT : STATUS_INVALID,
    }
  }
  let module: HeifModule
  try {
    module = await loadModule()
  } catch (error) {
    return isUnavailableError(error) ? { type: 'unavailable' } : { type: 'failed', status: 0 }
  }
  try {
    const bytes = new Uint8Array(await request.file.arrayBuffer())
    return decodeWithModule(module, bytes, request.maxPixels)
  } catch {
    return { type: 'failed', status: 0 }
  }
}

function isUnavailableError(error: unknown): boolean {
  return error instanceof Error && error.name === HEIF_CODEC_UNAVAILABLE
}
