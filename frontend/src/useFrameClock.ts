import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import type { StudioAnimation } from './processing/types'

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)'
/* GIF delays of 0 or 1 cs display as 10 ms, so the clock never schedules a zero-length frame. */
const MIN_FRAME_MS = 10

export interface FrameClockOptions {
  readonly animation: StudioAnimation | null
  /** Identity of the imported source; a different object restarts playback from frame 0. */
  readonly identity: object | null
  /** False while frames are still decoding; the clock holds its frame until they are ready. */
  readonly ready?: boolean
}

export interface FrameClock {
  /** Zero-based index shared by every view; always 0 without an animation. */
  readonly frame: number
  readonly playing: boolean
  /** Shows `frame` (clamped to the animation) and pauses playback. */
  setFrame(frame: number): void
  /** Starts playback; a play request after the final play restarts from frame 0. */
  setPlaying(playing: boolean): void
}

export interface FramePlayback extends FrameClock {
  readonly frameCount: number
}

interface ClockState {
  readonly identity: object | null
  readonly frame: number
  /** Completed loops since playback last started; compared with the GIF loop count. */
  readonly plays: number
  readonly playing: boolean
}

/** Playback starts unless the user asks for reduced motion, which starts paused on frame 0. */
function prefersReducedMotion(): boolean {
  if (typeof window.matchMedia !== 'function') return false
  return window.matchMedia(REDUCED_MOTION_QUERY).matches
}

function initialState(identity: object | null, animation: StudioAnimation | null): ClockState {
  return {
    identity,
    frame: 0,
    plays: 0,
    playing: animation !== null && !prefersReducedMotion(),
  }
}

function frameDurationMs(delaysCs: ReadonlyArray<number>, frame: number): number {
  return Math.max(MIN_FRAME_MS, (delaysCs[frame] ?? 0) * 10)
}

function clampFrame(frame: number, animation: StudioAnimation | null): number {
  if (animation === null || !Number.isFinite(frame)) return 0
  return Math.min(Math.max(0, Math.trunc(frame)), animation.frameCount - 1)
}

/**
 * Plays an animation's frames on one shared index. Elapsed time is accumulated from animation
 * frames against each frame's delay, so a late callback advances by the true time rather than by
 * one frame. Playback stops on the last frame once the loop count is spent. It holds while the
 * document is hidden or the frames are not decoded, resumes without a catch-up jump, and restarts
 * from frame 0 for a new source identity. Playback is cancelled on unmount.
 */
export function useFrameClock({
  animation,
  identity,
  ready = true,
}: FrameClockOptions): FrameClock {
  const [stored, setStored] = useState(() => initialState(identity, animation))
  const current = stored.identity === identity ? stored : initialState(identity, animation)
  if (current !== stored) setStored(current)

  const [hidden, setHidden] = useState(() => document.hidden)
  const schedule = useRef(animation)
  useEffect(() => {
    schedule.current = animation
  })
  useEffect(() => {
    const follow = () => setHidden(document.hidden)
    document.addEventListener('visibilitychange', follow)
    return () => document.removeEventListener('visibilitychange', follow)
  }, [])

  const frame = clampFrame(current.frame, animation)
  const playing = current.playing && animation !== null
  const running = playing && ready && !hidden
  const frameCount = animation?.frameCount ?? 0

  useEffect(() => {
    const plan = schedule.current
    if (!running || plan === null) return undefined
    const lastFrame = plan.frameCount - 1
    let index = frame
    let plays = current.plays
    let elapsed = 0
    let last: number | null = null
    let handle = 0
    const step = (now: number) => {
      if (last !== null) elapsed += now - last
      last = now
      let moved = false
      let finished = false
      while (elapsed >= frameDurationMs(plan.delaysCs, index)) {
        elapsed -= frameDurationMs(plan.delaysCs, index)
        if (index < lastFrame) {
          index += 1
        } else if (plays < plan.loopCount) {
          plays += 1
          index = 0
        } else {
          finished = true
          break
        }
        moved = true
      }
      if (finished) {
        setStored((previous) => ({ ...previous, frame: index, plays, playing: false }))
        return
      }
      if (moved) setStored((previous) => ({ ...previous, frame: index, plays }))
      handle = requestAnimationFrame(step)
    }
    handle = requestAnimationFrame(step)
    return () => cancelAnimationFrame(handle)
  }, [running, identity, frameCount])

  const setFrame = useCallback((next: number) => {
    setStored((previous) => ({
      ...previous,
      frame: clampFrame(next, schedule.current),
      plays: 0,
      playing: false,
    }))
  }, [])

  const setPlaying = useCallback((next: boolean) => {
    setStored((previous) => {
      const plan = schedule.current
      if (!next || plan === null)
        return previous.playing ? { ...previous, playing: false } : previous
      if (previous.playing) return previous
      const atEnd = previous.frame >= plan.frameCount - 1
      if (atEnd) return { ...previous, frame: 0, plays: 0, playing: true }
      return { ...previous, playing: true }
    })
  }, [])

  return { frame, playing, setFrame, setPlaying }
}

type FrameSet = ReadonlyArray<string>

const decodedSets = new WeakSet<FrameSet>()

function decodeImage(url: string): Promise<void> {
  if (typeof Image !== 'function') return Promise.resolve()
  const image = new Image()
  image.src = url
  if (typeof image.decode !== 'function') return Promise.resolve()
  return image.decode().then(
    () => undefined,
    () => undefined,
  )
}

/**
 * Tells whether the original and artwork frame sets have been decoded, so a swap between frames
 * never shows a blank image. Each set decodes once, one frame at a time, and a revoked URL counts
 * as decoded so it cannot hold playback forever.
 */
export function useDecodedFrames(
  first: FrameSet | undefined,
  second: FrameSet | undefined,
): boolean {
  const [, settled] = useReducer((count: number) => count + 1, 0)
  useEffect(() => {
    let active = true
    const pending = [first, second].filter(
      (set): set is FrameSet => set !== undefined && !decodedSets.has(set),
    )
    void pending.reduce<Promise<void>>(
      (chain, set) =>
        chain.then(async () => {
          if (!active) return
          for (const url of set) {
            if (!active) return
            await decodeImage(url)
          }
          decodedSets.add(set)
          if (active) settled()
        }),
      Promise.resolve(),
    )
    return () => {
      active = false
    }
  }, [first, second])
  return [first, second].every((set) => set === undefined || decodedSets.has(set))
}
