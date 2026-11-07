import { HEIF_CODEC_UNAVAILABLE } from '../heifDecodeProtocol'

/** Stands in for the Emscripten glue when `.local/wasm-codecs/dist` is absent; every load is refused. */
export default function createHeifDecoderModule(): Promise<never> {
  const error = new Error('HEIC and AVIF decoding is not included in this build.')
  error.name = HEIF_CODEC_UNAVAILABLE
  return Promise.reject(error)
}
