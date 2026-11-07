import { MAX_ENCODED_BYTES } from './protocol'
import type { BlobExportArtifact } from './types'
import type { ExportCompletion, ExportSink } from './workerClient'

/** Cumulative encoded-output ceiling; equal to the WASM encoded ceiling (256 MiB). */
export const MAX_EXPORT_BYTES = MAX_ENCODED_BYTES

export class ExportLimitError extends Error {
  readonly code = 'resource_limit_exceeded'

  constructor(limit: number) {
    super(`Export exceeds the ${limit.toLocaleString('en-US')} byte limit.`)
    this.name = 'ExportLimitError'
  }
}

export interface ExportAccumulatorOptions {
  readonly limitBytes?: number
  readonly onProgress?: (fraction: number) => void
}

/** Sink that owns the chunks of one export until it is finished or released. */
export interface ExportAccumulator extends ExportSink {
  /** Builds the `image/png` Blob and releases the chunks; fails if the totals disagree. */
  finish(completion: ExportCompletion): BlobExportArtifact
  /** Drops every accumulated chunk. Idempotent; later accepts fail. */
  release(): void
}

/**
 * Accumulates transferred chunks until `MAX_EXPORT_BYTES` (or `limitBytes`) would be
 * exceeded. Exceeding the limit releases the chunks and throws `ExportLimitError`.
 */
export function createExportAccumulator(options: ExportAccumulatorOptions = {}): ExportAccumulator {
  const limit = options.limitBytes ?? MAX_EXPORT_BYTES
  let chunks: ArrayBuffer[] | null = []
  let total = 0

  const release = (): void => {
    chunks = null
    total = 0
  }

  return {
    accept(chunk: ArrayBuffer): void {
      if (chunks === null) throw new Error('The export accumulator was released.')
      const next = total + chunk.byteLength
      if (!Number.isSafeInteger(next) || next > limit) {
        release()
        throw new ExportLimitError(limit)
      }
      chunks.push(chunk)
      total = next
    },
    progress(fraction: number): void {
      options.onProgress?.(fraction)
    },
    finish(completion: ExportCompletion): BlobExportArtifact {
      const parts = chunks
      if (parts === null) throw new Error('The export accumulator was released.')
      const matches = completion.chunkCount === parts.length && completion.byteLength === total
      release()
      if (!matches) throw new Error('The exported chunks do not match the worker totals.')
      const blob = new Blob(parts, { type: 'image/png' })
      return { kind: 'blob', blob, byteLength: blob.size, dispose: () => undefined }
    },
    release,
  }
}
