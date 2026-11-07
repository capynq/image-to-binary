import { useSyncExternalStore } from 'react'
import type { StudioController } from './state/controller'

/**
 * Binds a borrowed controller to React's concurrent-safe external-store subscription API. The
 * controller's lifetime belongs to the runtime that created it, so this hook never disposes it.
 */
export function useStudio(controller: StudioController): {
  readonly controller: StudioController
  readonly snapshot: ReturnType<StudioController['getSnapshot']>
} {
  const snapshot = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot,
  )
  return { controller, snapshot }
}
