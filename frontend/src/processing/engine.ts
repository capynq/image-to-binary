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
  startPreview(
    generation: number,
    cellSize: number,
    fontSize: number,
    threshold: number,
    sampling: PreviewSampling,
  ): EngineStage
  advance(generation: number, pixelBudget: number): EngineStage
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
}
