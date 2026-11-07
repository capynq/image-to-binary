import { createWorkerProcessingAdapter } from './processing/browserWorker'
import type { NativeFiles } from './processing/nativeFiles'
import type { NativeMenu } from './processing/nativeMenu'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'

/** The studio's controller and, inside the desktop shell, its native file entry points. */
export interface AppRuntime {
  readonly controller: StudioController
  readonly native: NativeFiles | null
  /** Present only inside the desktop shell, where the menu bar exists. */
  readonly menu?: NativeMenu | null
}

/**
 * True inside the Tauri webview. The desktop shell sets this flag itself, and the check is the
 * same one `@tauri-apps/api` performs, so the browser bundle never has to load Tauri code to ask.
 */
export function isNativeShell(): boolean {
  return (globalThis as { readonly isTauri?: unknown }).isTauri === true
}

/** Browser runtime: the worker adapter, created synchronously so the first render is not delayed. */
export function createBrowserRuntime(): AppRuntime {
  return {
    controller: createStudioController({ adapter: createWorkerProcessingAdapter() }),
    native: null,
  }
}
