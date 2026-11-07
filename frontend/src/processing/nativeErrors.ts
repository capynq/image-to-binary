import { ProcessingWorkerError } from './workerClient'

/** Shown when the shell rejects without a readable message. */
export const NATIVE_FALLBACK_MESSAGE = 'The native shell could not complete the request.'

/** Shown when a shell reply does not have the shape the page expects. */
export const NATIVE_MALFORMED_MESSAGE = 'The native shell sent a malformed reply.'

/** Maps an IPC rejection, `{ code, message }` from the shell, onto the page's typed error. */
export function nativeError(error: unknown): ProcessingWorkerError {
  if (error instanceof ProcessingWorkerError) return error
  if (typeof error === 'object' && error !== null) {
    const { code, message } = error as Record<string, unknown>
    if (typeof code === 'string') {
      return new ProcessingWorkerError(
        code,
        typeof message === 'string' && message !== '' ? message : NATIVE_FALLBACK_MESSAGE,
      )
    }
  }
  if (error instanceof Error && error.message !== '') {
    return new ProcessingWorkerError('internal', error.message)
  }
  return new ProcessingWorkerError('internal', NATIVE_FALLBACK_MESSAGE)
}

/** A malformed reply is an internal failure, reported with a fixed readable message. */
export function malformedReply(): ProcessingWorkerError {
  return new ProcessingWorkerError('internal', NATIVE_MALFORMED_MESSAGE)
}

/** Whether `value` is an integer count the page can represent exactly and that is not negative. */
export function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}
