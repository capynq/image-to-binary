import { Channel, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

/** Receiving end of a binary-safe progress channel; the native side streams messages into `onmessage`. */
export interface NativeChannel<T> {
  onmessage: (message: T) => void
}

/** The three Tauri entry points the native adapter and file layer need, injectable for tests. */
export interface NativeBridge {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>
  channel<T>(): NativeChannel<T>
  listen<T>(event: string, handler: (payload: T) => void): Promise<() => void>
}

/** Bridge over the official `@tauri-apps/api` package; only valid inside the Tauri webview. */
export function tauriBridge(): NativeBridge {
  return {
    invoke: (command, args) => invoke(command, args),
    channel: <T>() => new Channel<T>(),
    listen: async <T>(event: string, handler: (payload: T) => void) => {
      const unlisten = await listen<T>(event, (received) => handler(received.payload))
      return unlisten
    },
  }
}
