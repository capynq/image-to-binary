import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const css = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), '../styles.css'), 'utf8')
const ROLES = [
  'base',
  'surface',
  'panel',
  'panel-raised',
  'text',
  'muted',
  'faint',
  'accent',
  'accent-strong',
  'border',
  'control',
  'focus',
  'on-accent',
  'danger',
] as const
type Role = (typeof ROLES)[number]
type Theme = 'dark' | 'light'

function raw(theme: Theme, role: Role): string {
  const matches = [...css.matchAll(new RegExp(`--${theme}-${role}: (#[0-9a-f]{6});`, 'g'))]
  if (matches.length !== 1) throw new Error(`${theme} ${role} defined ${matches.length} times`)
  return matches[0]?.[1] ?? ''
}

function luminance(hex: string): number {
  const channels = [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255)
  const [r = 0, g = 0, b = 0] = channels.map((value) =>
    value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
  )
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function ratio(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return ((light ?? 0) + 0.05) / ((dark ?? 0) + 0.05)
}

const TEXT_ON: readonly Role[] = ['text', 'muted', 'faint', 'accent-strong', 'danger']
const SURFACES: readonly Role[] = ['base', 'surface', 'panel', 'panel-raised']

describe('graphite/cobalt tokens', () => {
  it('defines every raw colour exactly once per theme and no light-dark() remains', () => {
    for (const theme of ['dark', 'light'] as const) {
      for (const role of ROLES) expect(raw(theme, role)).toMatch(/^#[0-9a-f]{6}$/)
    }
    expect(css).not.toContain('light-dark(')
  })

  it('maps role tokens to raw values in one dark default and one light selector per condition', () => {
    expect(css.match(/--base: var\(--dark-base\);/g)).toHaveLength(1)
    expect(css.match(/--base: var\(--light-base\);/g)).toHaveLength(2)
  })

  it('scopes the system-light palette to the exact :root:not([data-theme=dark]) selector', () => {
    const selector = ":root:not([data-theme='dark'])"
    expect(css.split(selector)).toHaveLength(3)
    expect(css).toMatch(
      /@media \(prefers-color-scheme: light\) \{\s*:root:not\(\[data-theme='dark'\]\) \{\s*color-scheme: light;/,
    )
  })

  it('keeps text at WCAG AA (4.5:1) on every surface, in both themes', () => {
    for (const theme of ['dark', 'light'] as const) {
      for (const text of TEXT_ON) {
        for (const surface of SURFACES) {
          expect(ratio(raw(theme, text), raw(theme, surface))).toBeGreaterThanOrEqual(4.5)
        }
      }
      expect(ratio(raw(theme, 'on-accent'), raw(theme, 'accent'))).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('keeps focus rings and control boundaries at 3:1 against the surfaces they sit on', () => {
    for (const theme of ['dark', 'light'] as const) {
      for (const surface of ['base', 'surface'] as const) {
        expect(ratio(raw(theme, 'focus'), raw(theme, surface))).toBeGreaterThanOrEqual(3)
      }
      for (const surface of SURFACES) {
        expect(ratio(raw(theme, 'control'), raw(theme, surface))).toBeGreaterThanOrEqual(3)
      }
    }
  })

  it('uses no :has() selector, so focus rings work without it', () => {
    expect(css).not.toContain(':has(')
    expect(css).not.toContain(':focus-within')
    expect(css).not.toMatch(/[\w\]]:focus-visible/)
    expect(css).toMatch(/\.mode-option input:focus,\n/)
  })
})
