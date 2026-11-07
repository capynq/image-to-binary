import type { MenuState } from './processing/nativeMenu'

/** Studio actions that the keyboard and the desktop menu can request. */
export type StudioAction = 'open' | 'export' | 'original' | 'artwork' | 'render' | 'cancel'

/** Facts the gates read; built once per render so the buttons and the menu share one answer. */
export interface StudioFacts {
  readonly hasSource: boolean
  readonly busy: boolean
  readonly draftBlocked: boolean
}

export interface StudioGates {
  readonly open: boolean
  readonly export: boolean
  readonly render: boolean
  readonly original: boolean
  readonly artwork: boolean
  readonly cancel: boolean
}

/**
 * The single source of truth for when each action may run. A missing source, a busy studio
 * (import, render or export) or an invalid draft closes the gate; the menu state and the
 * buttons both read these flags.
 */
export function studioGates({ hasSource, busy, draftBlocked }: StudioFacts): StudioGates {
  return {
    open: true,
    export: hasSource && !busy && !draftBlocked,
    render: hasSource && !busy && !draftBlocked,
    original: hasSource,
    artwork: hasSource && !draftBlocked,
    cancel: busy,
  }
}

/** Menu enabled flags derived from the gates; `open` is always enabled and has no menu flag. */
export function menuStateOf(gates: StudioGates): MenuState {
  return {
    export: gates.export,
    render: gates.render,
    original: gates.original,
    artwork: gates.artwork,
    cancel: gates.cancel,
  }
}

export type StudioHandlers = Readonly<Record<StudioAction, () => void>>

/**
 * Runs a studio action only through its gate. Returns whether the handler ran, so a refused
 * menu item or Escape press is a silent no-op rather than a partial state change.
 */
export function runStudioAction(
  action: StudioAction,
  gates: StudioGates,
  handlers: StudioHandlers,
): boolean {
  if (!gates[action]) return false
  handlers[action]()
  return true
}

const TEXT_INPUT_TYPES: ReadonlySet<string> = new Set([
  '',
  'text',
  'search',
  'url',
  'tel',
  'email',
  'password',
  'number',
])

/**
 * True when keystrokes would type into the element, so no shortcut may claim them. Duck-typed so
 * it works for any element, including ones from another realm or a test double.
 */
export function isTextEntry(target: unknown): boolean {
  if (typeof target !== 'object' || target === null) return false
  const element = target as {
    readonly tagName?: unknown
    readonly type?: unknown
    readonly isContentEditable?: unknown
  }
  if (element.isContentEditable === true) return true
  if (element.tagName === 'TEXTAREA') return true
  if (element.tagName !== 'INPUT') return false
  const type = typeof element.type === 'string' ? element.type.toLowerCase() : ''
  return TEXT_INPUT_TYPES.has(type)
}

/** Minimal keyboard event shape the shortcut mapping reads. */
export interface ShortcutKey {
  readonly key: string
  readonly altKey: boolean
  readonly ctrlKey: boolean
  readonly metaKey: boolean
  readonly shiftKey: boolean
  readonly target: unknown
}

/**
 * Maps a page keystroke to a studio action. The page handles only Escape, which cancels. Every
 * accelerator with a modifier belongs to the desktop menu, and the browser keeps its own
 * shortcuts for those keys (Ctrl/Cmd+O, E, R, 1 and 2 are reserved or used by the browser), so
 * the browser relies on the buttons for them. Escape never fires while typing in a field.
 */
export function shortcutAction(event: ShortcutKey): StudioAction | null {
  if (event.key !== 'Escape') return null
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return null
  if (isTextEntry(event.target)) return null
  return 'cancel'
}

/** True when focus is on nothing useful: the body, nothing at all, a detached node or a fallback marker. */
export function isFocusLost(active: Element | null): boolean {
  if (active === null || !active.isConnected) return true
  if (active === active.ownerDocument.body) return true
  return active.hasAttribute('data-focus-fallback')
}

/**
 * True when the element can take focus now: attached, enabled, and neither hidden nor inert. A
 * control that is disabled or hidden when focus is restored would otherwise drop focus to body.
 */
export function isReachable(element: HTMLElement | null): element is HTMLElement {
  if (element === null || !element.isConnected) return false
  if ((element as HTMLElement & { readonly disabled?: boolean }).disabled === true) return false
  if (element.closest('[hidden], [inert]') !== null) return false
  const style = getComputedStyle(element)
  return style.display !== 'none' && style.visibility !== 'hidden'
}

/** Focuses the target when reachable, otherwise the fallback; returns what was focused, or null. */
export function focusReachable(
  target: HTMLElement | null,
  fallback: HTMLElement | null,
): HTMLElement | null {
  const chosen = isReachable(target) ? target : isReachable(fallback) ? fallback : null
  chosen?.focus()
  return chosen
}
