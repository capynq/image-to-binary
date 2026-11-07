import createHeifDecoderModule from 'heif-decoder-glue'
import { isHeifDecodeRequest } from './heifDecodeProtocol'
import type { HeifDecodeReply } from './heifDecodeProtocol'
import { runHeifDecode } from './heifDecoderCore'

interface DecoderScope {
  postMessage(message: HeifDecodeReply, transfer: Transferable[]): void
  addEventListener(type: 'message', listener: (event: MessageEvent<unknown>) => void): void
}

const scope = self as unknown as DecoderScope

scope.addEventListener('message', (event) => {
  const request: unknown = event.data
  if (!isHeifDecodeRequest(request)) {
    scope.postMessage({ type: 'failed', status: 0 }, [])
    return
  }
  void runHeifDecode(createHeifDecoderModule, request).then((reply) => {
    scope.postMessage(reply, reply.type === 'decoded' ? [reply.pixels] : [])
  })
})
