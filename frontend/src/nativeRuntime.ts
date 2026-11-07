import type { AppRuntime } from './runtime'
import { createNativeProcessingAdapter } from './processing/nativeAdapter'
import { tauriBridge } from './processing/nativeBridge'
import { createNativeFiles } from './processing/nativeFiles'
import { createNativeMenu } from './processing/nativeMenu'
import { createStudioController } from './state/controller'

/**
 * Desktop runtime. It is loaded only when the shell is detected, so the Tauri bridge and the
 * native adapter are never part of the browser's bundle.
 */
export function createNativeRuntime(): AppRuntime {
  const bridge = tauriBridge()
  return {
    controller: createStudioController({ adapter: createNativeProcessingAdapter({ bridge }) }),
    native: createNativeFiles(bridge),
    menu: createNativeMenu(bridge),
  }
}
