import { useCallback, useEffect, useRef, useState } from 'react'
import type { ChangeEvent, DragEvent, RefObject } from 'react'
import { importNativeFile, toInput } from './processing/nativeFiles'
import type { NativeFiles } from './processing/nativeFiles'
import type { PreviewKind } from './processing/types'
import type { StudioController } from './state/controller'

export const IMPORT_ACCEPT =
  '.avif,.bmp,.gif,.heic,.heif,.jpg,.jpeg,.png,.tif,.tiff,.webp,image/avif,image/bmp,image/gif,image/heic,image/heif,image/jpeg,image/png,image/tiff,image/webp'

export const IMPORT_GUIDANCE =
  'Use one AVIF, BMP, GIF, HEIC, HEIF, JPEG, PNG, TIFF or WebP image up to 256 MiB and 50 megapixels, with a file name of at most 256 characters.'

/** Formats an encoded byte count for display using binary units. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
}

export interface ImportFlow {
  readonly inputRef: RefObject<HTMLInputElement | null>
  readonly view: PreviewKind
  readonly localError: string | null
  readonly dragActive: boolean
  openPicker(): void
  onInputChange(event: ChangeEvent<HTMLInputElement>): void
  loadDemo(): void
  selectView(kind: PreviewKind): void
  renderArtwork(): void
  cancel(): void
  reset(): void
  onDragEnter(event: DragEvent<HTMLElement>): void
  onDragOver(event: DragEvent<HTMLElement>): void
  onDragLeave(event: DragEvent<HTMLElement>): void
  onDrop(event: DragEvent<HTMLElement>): void
}

function carriesFiles(event: DragEvent<HTMLElement>): boolean {
  return Array.from(event.dataTransfer.types).includes('Files')
}

/**
 * UI-side import intent for one borrowed controller. Each import, cancel, or reset
 * advances an intent token; a completed import starts its original preview only when
 * the token is unchanged, the controller is ready, and the source metadata identity
 * differs from the one captured before the import began. Only then is the original
 * view selected, before the preview is awaited; failed, cancelled, or superseded
 * imports leave the current view untouched. Changing the controller invalidates any
 * pending intent. Dropped data is read only through `dataTransfer.files`.
 */
export function useImportFlow(
  controller: StudioController,
  native: NativeFiles | null = null,
): ImportFlow {
  const inputRef = useRef<HTMLInputElement>(null)
  const intent = useRef(0)
  const dragDepth = useRef(0)
  const [view, setView] = useState<PreviewKind>('original')
  const [localError, setLocalError] = useState<string | null>(null)
  const [dragActive, setDragActive] = useState(false)

  useEffect(
    () => () => {
      intent.current += 1
    },
    [controller],
  )

  const runImport = useCallback(
    async (start: () => Promise<void>): Promise<void> => {
      const token = ++intent.current
      const before = controller.getSnapshot().metadata
      setLocalError(null)
      try {
        await start()
      } catch {
        return
      }
      if (intent.current !== token) return
      const after = controller.getSnapshot()
      if (after.status !== 'ready' || after.metadata === null || after.metadata === before) return
      setView('original')
      try {
        await controller.preview('original')
      } catch {
        return
      }
    },
    [controller],
  )

  const importFiles = useCallback(
    (files: readonly File[]): void => {
      const [file] = files
      if (file === undefined) return
      if (files.length > 1) {
        setLocalError(
          `Choose one image at a time. ${files.length} files were selected, so nothing was changed.`,
        )
        return
      }
      void runImport(() => controller.import(file))
    },
    [controller, runImport],
  )

  const openPicker = useCallback((): void => {
    if (native === null) {
      inputRef.current?.click()
      return
    }
    native.openImage().then(
      (file) => {
        if (file !== null) void runImport(() => importNativeFile(controller, native, file))
      },
      (error: unknown) => setLocalError(error instanceof Error ? error.message : String(error)),
    )
  }, [controller, native, runImport])

  useEffect(() => {
    if (native === null) return undefined
    let attached = true
    const stops: Array<() => void> = []
    const attach = (subscription: Promise<() => void>): void => {
      subscription
        .then((stop) => {
          if (attached) stops.push(stop)
          else stop()
        })
        .catch(() => undefined)
    }
    attach(
      native.listenDrops((outcome) => {
        dragDepth.current = 0
        setDragActive(false)
        if (outcome.status === 'accepted') {
          void runImport(() => importNativeFile(controller, native, toInput(outcome.file)))
        } else {
          setLocalError(outcome.error.message)
        }
      }),
    )
    attach(
      native.listenHover((active) => {
        if (!active) dragDepth.current = 0
        setDragActive(active)
      }),
    )
    return () => {
      attached = false
      for (const stop of stops) stop()
    }
  }, [controller, native, runImport])

  const onInputChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>): void => {
      const input = event.currentTarget
      const files = Array.from(input.files ?? [])
      input.value = ''
      importFiles(files)
    },
    [importFiles],
  )

  const loadDemo = useCallback((): void => {
    void runImport(() => controller.loadDemo())
  }, [controller, runImport])

  const selectView = useCallback(
    (kind: PreviewKind): void => {
      setView(kind)
      const current = controller.getSnapshot()
      const cached = kind === 'original' ? current.original : current.preview
      if (cached !== null || current.metadata === null || current.status === 'importing') return
      controller.preview(kind).catch(() => undefined)
    },
    [controller],
  )

  const renderArtwork = useCallback((): void => {
    setView('artwork')
    const current = controller.getSnapshot()
    if (current.metadata === null || current.status === 'importing') return
    controller.preview('artwork').catch(() => undefined)
  }, [controller])

  const cancel = useCallback((): void => {
    intent.current += 1
    controller.cancel()
  }, [controller])

  const reset = useCallback((): void => {
    intent.current += 1
    dragDepth.current = 0
    setDragActive(false)
    setLocalError(null)
    setView('original')
    controller.reset()
  }, [controller])

  const onDragEnter = useCallback((event: DragEvent<HTMLElement>): void => {
    if (!carriesFiles(event)) return
    event.preventDefault()
    dragDepth.current += 1
    setDragActive(true)
  }, [])

  const onDragOver = useCallback((event: DragEvent<HTMLElement>): void => {
    event.preventDefault()
    event.dataTransfer.dropEffect = carriesFiles(event) ? 'copy' : 'none'
  }, [])

  const onDragLeave = useCallback((): void => {
    if (dragDepth.current === 0) return
    dragDepth.current -= 1
    if (dragDepth.current === 0) setDragActive(false)
  }, [])

  const onDrop = useCallback(
    (event: DragEvent<HTMLElement>): void => {
      event.preventDefault()
      dragDepth.current = 0
      setDragActive(false)
      const files = Array.from(event.dataTransfer.files)
      if (files.length === 0) {
        setLocalError('Drop an image file. Links and text cannot be opened.')
        return
      }
      importFiles(files)
    },
    [importFiles],
  )

  return {
    inputRef,
    view,
    localError,
    dragActive,
    openPicker,
    onInputChange,
    loadDemo,
    selectView,
    renderArtwork,
    cancel,
    reset,
    onDragEnter,
    onDragOver,
    onDragLeave,
    onDrop,
  }
}
