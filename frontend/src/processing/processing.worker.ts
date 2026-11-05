import init, { ImageSession, JobStage } from '../../../.local/wasm/pkg/image_to_binary_wasm.js'
import type { ProcessingEngine } from './engine'
import { createWorkerHost } from './workerHost'
import type { HostEndpoint } from './workerHost'

async function loadEngine(): Promise<ProcessingEngine> {
  await init()
  return {
    readyStage: JobStage.Ready,
    failedStage: JobStage.Failed,
    createSession: (width, height, pixels) => ImageSession.fromRgba(width, height, pixels),
  }
}

createWorkerHost(self as unknown as HostEndpoint, loadEngine)
