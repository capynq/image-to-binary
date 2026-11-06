import type { PreviewSampling } from './protocol'

export type EngineStage = number

/** The subset of `PreviewFrame` the worker consumes. */
export interface EngineFrame {
  readonly width: number
  readonly height: number
  /**
   * Returns a fresh RGBA8 buffer owned by the caller, never a view of WASM memory,
   * so its `ArrayBuffer` may be transferred. The wrapper must still be freed.
   */
  takePixels(): Uint8Array<ArrayBuffer>
  free(): void
}

/** The subset of `ImageSession` the worker drives; the generated class satisfies it. */
export interface EngineSession {
  /** Immutable post-orientation width in pixels. */
  readonly sourceWidth: number
  /** Immutable post-orientation height in pixels. */
  readonly sourceHeight: number
  /** Normalized channel count; always 4 (RGBA8). */
  readonly sourceChannels: number
  startPreview(
    generation: number,
    cellSize: number,
    fontSize: number,
    threshold: number,
    sampling: PreviewSampling,
  ): EngineStage
  /** Starts the bounded original-image preview job; shares advance, cancel and finish. */
  startOriginalPreview(generation: number): EngineStage
  /** Starts a PNG export that samples the same grid as a preview with the same settings. */
  startExport(
    generation: number,
    cellSize: number,
    fontSize: number,
    threshold: number,
    sampling: PreviewSampling,
  ): EngineStage
  advance(generation: number, pixelBudget: number): EngineStage
  /** Returns encoded chunks completed since the last drain; each array is JS-owned. */
  drainExport(generation: number): Uint8Array[]
  /** Returns the chunks not yet drained; the job is released on success. */
  finishExport(generation: number): Uint8Array[]
  progressCompleted(): number
  progressTotal(): number
  cancel(generation: number): void
  finish(generation: number): EngineFrame
  dispose(): void
  free(): void
}

/** Abstracts the generated WASM package so the worker host is testable without a build. */
export interface ProcessingEngine {
  readonly readyStage: EngineStage
  readonly failedStage: EngineStage
  createSession(width: number, height: number, pixels: Uint8Array): EngineSession
  /** Decodes encoded image bytes synchronously; throws coded errors on failure. */
  createEncodedSession(bytes: Uint8Array): EngineSession
}
