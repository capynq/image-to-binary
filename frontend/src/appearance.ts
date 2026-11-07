export type Theme = 'dark' | 'light'
/** The appearance the user chose; `system` follows the operating system and is stored explicitly. */
export type ThemeChoice = 'system' | Theme

export const THEME_CHOICE_KEY = 'image-to-binary-theme-choice'
export const LIGHT_SCHEME_QUERY = '(prefers-color-scheme: light)'

/** Fallback for the window's theme colour when the stylesheet's `--base` token is unavailable. */
export const THEME_COLOUR: Readonly<Record<Theme, string>> = {
  dark: '#0e1118',
  light: '#f3f5fa',
}

/**
 * The stored appearance choice. Anything missing, unreadable or unknown means `system`. The
 * pre-5.3 key is never read: it was written on every load, so it cannot tell a real choice apart.
 */
export function readThemeChoice(): ThemeChoice {
  try {
    const stored = window.localStorage.getItem(THEME_CHOICE_KEY)
    if (stored === 'light' || stored === 'dark' || stored === 'system') return stored
  } catch {
    return 'system'
  }
  return 'system'
}

export function writeThemeChoice(choice: ThemeChoice): void {
  try {
    window.localStorage.setItem(THEME_CHOICE_KEY, choice)
  } catch {
    return
  }
}

export function currentSystemTheme(): Theme {
  if (typeof window.matchMedia !== 'function') return 'dark'
  return window.matchMedia(LIGHT_SCHEME_QUERY).matches ? 'light' : 'dark'
}
