import { createBrowserProcessingAdapter } from './browserAdapter'
import type { ProcessingAdapter } from './types'
import { createProcessingClient } from './workerClient'

/** Production adapter: a lazily created module worker running the Rust WASM engine. */
export function createWorkerProcessingAdapter(): ProcessingAdapter {
  return createBrowserProcessingAdapter({
    createClient: () =>
      createProcessingClient({
        createEndpoint: () =>
          new Worker(new URL('./processing.worker.ts', import.meta.url), { type: 'module' }),
      }),
  })
}
