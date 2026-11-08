import { useEffect, useRef, useState } from 'react'
import type { StudioController } from './state/controller'
import type { StudioSnapshot } from './processing/types'

/** Trailing delay between the last accepted settings change and the quiet artwork render it starts. */
export const AUTO_RENDER_DELAY_MS = 250

export interface AutoRenderOptions {
  readonly controller: StudioController
  readonly snapshot: StudioSnapshot
  /** True while a draft is invalid; nothing renders automatically until the draft is fixed. */
  readonly draftBlocked: boolean
  /** Starts a quiet artwork render through the import flow, which keeps the view and the status. */
  readonly renderQuietly: () => void
}

export interface AutoRender {
  /** True while a render is armed and has neither fired nor been dropped. */
  readonly pending: boolean
  /** Drops an armed render; a manual render calls this before it starts. */
  clear(): void
  /** Escape: drops an armed render and withholds automatic renders until the next settings change. */
  suppress(): void
}

function artworkNeedsRender(snapshot: StudioSnapshot): boolean {
  return snapshot.preview === null || snapshot.previewRevision !== snapshot.settingsRevision
}

/**
 * Keeps the artwork current without a manual render. An accepted settings change arms a trailing
 * timer; when it fires, a quiet render runs only if a source is loaded, the status is ready, no
 * draft is invalid and the artwork is stale or missing. A completed import renders at once after
 * its original preview settles, and an export that ends with stale artwork re-arms the timer.
 *
 * Each settings revision is retired once a render has been attempted or suppressed, so a failed or
 * cancelled render is not retried until the next change. Exports and manual renders drop the timer.
 */
export function useAutoRender({
  controller,
  snapshot,
  draftBlocked,
  renderQuietly,
}: AutoRenderOptions): AutoRender {
  const [pending, setPending] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const latest = useRef({ controller, draftBlocked, renderQuietly })
  const retired = useRef<number | null>(null)
  const seenRevision = useRef(snapshot.settingsRevision)
  const seenMetadata = useRef(snapshot.metadata)
  const seenStatus = useRef(snapshot.status)
  const awaitingOriginal = useRef(false)

  useEffect(() => {
    latest.current = { controller, draftBlocked, renderQuietly }
  })

  const clearTimer = (): void => {
    if (timer.current !== null) clearTimeout(timer.current)
    timer.current = null
  }

  const eligible = (current: StudioSnapshot): boolean =>
    current.metadata !== null &&
    !latest.current.draftBlocked &&
    retired.current !== current.settingsRevision &&
    artworkNeedsRender(current)

  const fire = (): void => {
    timer.current = null
    setPending(false)
    const current = latest.current.controller.getSnapshot()
    if (current.status !== 'ready' || !eligible(current)) return
    retired.current = current.settingsRevision
    latest.current.renderQuietly()
  }

  const arm = (): void => {
    clearTimer()
    setPending(true)
    timer.current = setTimeout(fire, AUTO_RENDER_DELAY_MS)
  }

  useEffect(() => {
    if (snapshot.settingsRevision === seenRevision.current) return
    seenRevision.current = snapshot.settingsRevision
    clearTimer()
    if (eligible(snapshot)) arm()
    else setPending(false)
  }, [snapshot.settingsRevision])

  useEffect(() => {
    if (snapshot.metadata !== seenMetadata.current) {
      seenMetadata.current = snapshot.metadata
      awaitingOriginal.current = snapshot.metadata !== null
    }
    if (snapshot.status === 'error') awaitingOriginal.current = false
    if (!awaitingOriginal.current || snapshot.original === null || snapshot.status !== 'ready') {
      return
    }
    awaitingOriginal.current = false
    clearTimer()
    setPending(false)
    if (latest.current.draftBlocked) return
    retired.current = snapshot.settingsRevision
    latest.current.renderQuietly()
  }, [snapshot.metadata, snapshot.original, snapshot.status, snapshot.settingsRevision])

  useEffect(() => {
    const previous = seenStatus.current
    seenStatus.current = snapshot.status
    if (snapshot.status === 'exporting') {
      clearTimer()
      setPending(false)
      return
    }
    if (previous === 'exporting' && snapshot.status === 'ready' && eligible(snapshot)) arm()
  }, [snapshot.status])

  useEffect(() => () => clearTimer(), [])

  return {
    pending,
    clear: () => {
      clearTimer()
      setPending(false)
    },
    suppress: () => {
      clearTimer()
      setPending(false)
      retired.current = latest.current.controller.getSnapshot().settingsRevision
    },
  }
}
