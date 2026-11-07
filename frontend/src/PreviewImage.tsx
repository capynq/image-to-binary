import { useEffect, useRef, useState } from 'react'
import type { CSSProperties } from 'react'

export interface PreviewImageProps {
  readonly src: string
  readonly alt: string
  readonly width: number
  readonly height: number
  /** Inline sizing for callers that place the image in a scaled stage; the box is measured from it. */
  readonly style?: CSSProperties
}

/**
 * Media query that stops matching as soon as the device pixel ratio leaves `ratio`. The min and max
 * pair keeps the query exact for fractional ratios such as 1.7999999523, which a bare `resolution`
 * feature compares less predictably.
 */
export function resolutionQuery(ratio: number): string {
  return `(min-resolution: ${ratio}dppx) and (max-resolution: ${ratio}dppx)`
}

/**
 * Reports the current device pixel ratio and follows changes, such as a window moving between
 * displays with different scale factors. The listener re-arms on every change, and a resize
 * re-reads the ratio as a backstop for platforms that report a scale change only through resize.
 */
export function useDevicePixelRatio(): number {
  const [ratio, setRatio] = useState(readRatio)
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return undefined
    let query: MediaQueryList | null = null
    const follow = () => {
      setRatio(readRatio())
      arm()
    }
    const arm = () => {
      query?.removeEventListener('change', follow)
      query = window.matchMedia(resolutionQuery(readRatio()))
      query.addEventListener('change', follow)
    }
    arm()
    window.addEventListener('resize', follow)
    return () => {
      query?.removeEventListener('change', follow)
      window.removeEventListener('resize', follow)
    }
  }, [])
  return ratio
}

function readRatio(): number {
  const ratio = window.devicePixelRatio
  return Number.isFinite(ratio) && ratio > 0 ? ratio : 1
}

/**
 * True when the preview's displayed scale, multiplied by the device pixel ratio, enlarges the
 * image's pixels. Binary artwork then needs nearest-neighbour sampling to stay crisp; downscaled
 * previews keep the browser's smoothing so they do not alias.
 */
export function isUpscaledOnDevice(
  box: { readonly width: number; readonly height: number },
  natural: { readonly width: number; readonly height: number },
  ratio: number,
): boolean {
  if (natural.width <= 0 || natural.height <= 0 || box.width <= 0 || box.height <= 0) return false
  const scale = Math.min(box.width / natural.width, box.height / natural.height)
  return scale * ratio > 1
}

/**
 * Preview image that keeps its binary artwork crisp at every device pixel ratio. Its box is measured
 * with a ResizeObserver, and the sampling mode is recomputed when the ratio changes, so a display
 * move re-renders the same image with no state loss.
 */
export function PreviewImage({ src, alt, width, height, style }: PreviewImageProps) {
  const ref = useRef<HTMLImageElement>(null)
  const ratio = useDevicePixelRatio()
  const [box, setBox] = useState<{ width: number; height: number } | null>(null)

  useEffect(() => {
    const element = ref.current
    if (element === null || typeof ResizeObserver !== 'function') return undefined
    const observer = new ResizeObserver(() => {
      const rect = element.getBoundingClientRect()
      setBox({ width: rect.width, height: rect.height })
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [src])

  const upscaled = box !== null && isUpscaledOnDevice(box, { width, height }, ratio)
  return (
    <img
      ref={ref}
      className="fixture-preview"
      src={src}
      alt={alt}
      width={width}
      height={height}
      data-upscaled={upscaled ? 'true' : undefined}
      draggable={false}
      style={style}
    />
  )
}
