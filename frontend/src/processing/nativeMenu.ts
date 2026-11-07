import type { NativeBridge } from './nativeBridge'

export const NATIVE_MENU_EVENT = 'native-menu-action'

/** Studio actions a menu item can request; each one is gated by the same rules as its button. */
export const MENU_ACTIONS = [
  'open',
  'export',
  'original',
  'artwork',
  'split',
  'render',
  'cancel',
] as const
export type MenuAction = (typeof MENU_ACTIONS)[number]

/** Enabled flags the page reports to the menu bar. Every flag is required. */
export interface MenuState {
  readonly export: boolean
  readonly render: boolean
  readonly original: boolean
  readonly artwork: boolean
  readonly split: boolean
  readonly cancel: boolean
}

/** The menu bar as the page sees it: it receives actions and reports enabled state, nothing else. */
export interface NativeMenu {
  /** Rejects when the shell refuses the state; callers keep the menu as it was. */
  setState(state: MenuState): Promise<void>
  listen(handler: (action: MenuAction) => void): Promise<() => void>
}

/** Narrows a menu payload to a known action; anything else is ignored rather than guessed at. */
export function readMenuAction(payload: unknown): MenuAction | null {
  if (typeof payload !== 'object' || payload === null) return null
  const action = (payload as { readonly action?: unknown }).action
  return MENU_ACTIONS.find((known) => known === action) ?? null
}

/** Menu adapter over the Tauri bridge; only valid inside the desktop shell. */
export function createNativeMenu(bridge: NativeBridge): NativeMenu {
  return {
    setState: async (state) => {
      await bridge.invoke('set_menu_state', {
        state: {
          export: state.export,
          render: state.render,
          original: state.original,
          artwork: state.artwork,
          split: state.split,
          cancel: state.cancel,
        },
      })
    },
    listen: (handler) =>
      bridge.listen<unknown>(NATIVE_MENU_EVENT, (payload) => {
        const action = readMenuAction(payload)
        if (action !== null) handler(action)
      }),
  }
}

/**
 * Pushes the enabled state, retrying once on the next tick when the shell refuses it. A second
 * refusal is reported to the console and the menu keeps its previous state. The returned function
 * cancels a pending retry, so a superseded state is never re-sent over a newer one.
 */
export function syncMenuState(
  menu: NativeMenu,
  state: MenuState,
  report: (error: unknown) => void = (error) => console.error('set_menu_state was refused', error),
): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null
  const push = (retriesLeft: number): void => {
    menu.setState(state).catch((error: unknown) => {
      if (retriesLeft > 0) {
        timer = setTimeout(() => push(retriesLeft - 1), 0)
        return
      }
      report(error)
    })
  }
  push(1)
  return () => {
    if (timer !== null) clearTimeout(timer)
  }
}
