import { useEffect, useRef, useSyncExternalStore } from 'react'
import { createStudioController } from './state/controller'
import type { StudioController } from './state/controller'

/**
 * Binds a controller to React's concurrent-safe external-store subscription API.
 * A supplied controller is borrowed and never disposed by this hook; an omitted
 * controller is owned by the hook and disposed after its final unmount.
 */
export function useStudio(controller?: StudioController): {
  readonly controller: StudioController
  readonly snapshot: ReturnType<StudioController['getSnapshot']>
  readonly loadDemo: StudioController['loadDemo']
  readonly import: StudioController['import']
  readonly updateSettings: StudioController['updateSettings']
  readonly preview: StudioController['preview']
  readonly render: StudioController['preview']
  readonly exportDemo: StudioController['exportDemo']
  readonly reset: StudioController['reset']
  readonly cancel: StudioController['cancel']
  readonly dispose: StudioController['dispose']
} {
  const ownedController = useRef<StudioController | null>(null)
  if (controller === undefined && ownedController.current === null) {
    ownedController.current = createStudioController()
  }
  const activeController = controller ?? ownedController.current
  if (activeController === null) {
    throw new Error('Studio controller could not be created.')
  }
  const lifecycle = useRef(0)
  const snapshot = useSyncExternalStore(
    activeController.subscribe,
    activeController.getSnapshot,
    activeController.getSnapshot,
  )
  useEffect(() => {
    if (controller !== undefined) return undefined
    const token = ++lifecycle.current
    return () => {
      queueMicrotask(() => {
        if (lifecycle.current === token) {
          activeController.dispose()
          if (ownedController.current === activeController) {
            ownedController.current = null
          }
        }
      })
    }
  }, [activeController, controller])
  return {
    controller: activeController,
    snapshot,
    loadDemo: activeController.loadDemo,
    import: activeController.import,
    updateSettings: activeController.updateSettings,
    preview: activeController.preview,
    render: activeController.preview,
    exportDemo: activeController.exportDemo,
    reset: activeController.reset,
    cancel: activeController.cancel,
    dispose: activeController.dispose,
  }
}
