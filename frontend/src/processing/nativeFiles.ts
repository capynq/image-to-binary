import type { StudioController } from '../state/controller'
import type { NativeBridge } from './nativeBridge'
import { isCount, malformedReply, nativeError, NATIVE_MALFORMED_MESSAGE } from './nativeErrors'
import type { NativeFileInput } from './types'

/** Native file selection as the shell reports it: an opaque handle, a display name and a size. */
export interface FileReply {
  readonly id: number
  readonly name: string
  readonly byteLength: number
}

/** Payload of the `native-file-drop` event, produced after the shell checked the drop. */
export type NativeDropOutcome =
  | { readonly status: 'accepted'; readonly file: FileReply }
  | {
      readonly status: 'rejected'
      readonly error: { readonly code: string; readonly message: string }
    }

export const NATIVE_DROP_EVENT = 'native-file-drop'
export const NATIVE_HOVER_EVENT = 'native-file-hover'

/** Native file entry points used by the studio: dialog, handle release, and window drop/hover events. */
export interface NativeFiles {
  /** Resolves to `null` when the user cancels; a cancel is not an error. */
  openImage(): Promise<NativeFileInput | null>
  /** Releases a handle the page no longer owns. Idempotent and never rejects. */
  release(id: number): Promise<void>
  listenDrops(handler: (outcome: NativeDropOutcome) => void): Promise<() => void>
  listenHover(handler: (active: boolean) => void): Promise<() => void>
}

/** Validates a file reply from the shell; anything else is a malformed reply. */
export function readFileReply(value: unknown): FileReply {
  if (typeof value !== 'object' || value === null) throw malformedReply()
  const { id, name, byteLength } = value as Record<string, unknown>
  if (!isCount(id) || typeof name !== 'string' || !isCount(byteLength)) throw malformedReply()
  return { id, name, byteLength }
}

/** Validates a drop payload; a malformed payload becomes a rejected drop with a readable message. */
export function readDropOutcome(payload: unknown): NativeDropOutcome {
  const rejected = (): NativeDropOutcome => ({
    status: 'rejected',
    error: { code: 'internal', message: NATIVE_MALFORMED_MESSAGE },
  })
  if (typeof payload !== 'object' || payload === null) return rejected()
  const record = payload as Record<string, unknown>
  if (record.status === 'accepted') {
    try {
      return { status: 'accepted', file: readFileReply(record.file) }
    } catch {
      return rejected()
    }
  }
  if (record.status === 'rejected' && typeof record.error === 'object' && record.error !== null) {
    const { code, message } = record.error as Record<string, unknown>
    if (typeof code === 'string' && typeof message === 'string') {
      return { status: 'rejected', error: { code, message } }
    }
  }
  return rejected()
}

export function createNativeFiles(bridge: NativeBridge): NativeFiles {
  return {
    async openImage(): Promise<NativeFileInput | null> {
      let reply: unknown
      try {
        reply = await bridge.invoke<unknown>('open_image_dialog')
      } catch (error) {
        throw nativeError(error)
      }
      return reply === null ? null : toInput(readFileReply(reply))
    },
    release: (id) =>
      bridge.invoke('release_handle', { id }).then(
        () => undefined,
        () => undefined,
      ),
    listenDrops: (handler) =>
      bridge.listen<unknown>(NATIVE_DROP_EVENT, (payload) => handler(readDropOutcome(payload))),
    listenHover: (handler) =>
      bridge.listen<unknown>(NATIVE_HOVER_EVENT, (payload) => {
        const active = (payload as { readonly active?: unknown } | null)?.active
        if (typeof active === 'boolean') handler(active)
      }),
  }
}

/** Converts an accepted native file into the opaque import input the adapter accepts. */
export function toInput(file: FileReply): NativeFileInput {
  return { kind: 'native-file', id: file.id, name: file.name, byteLength: file.byteLength }
}

/**
 * Imports a native file, or releases its handle when the controller is already disposed. Such an
 * import never reaches the adapter, which otherwise owns the handle's release.
 */
export async function importNativeFile(
  controller: Pick<StudioController, 'getSnapshot' | 'import'>,
  files: Pick<NativeFiles, 'release'>,
  file: NativeFileInput,
): Promise<void> {
  if (controller.getSnapshot().status === 'disposed') {
    await files.release(file.id)
    return
  }
  await controller.import(file)
}
