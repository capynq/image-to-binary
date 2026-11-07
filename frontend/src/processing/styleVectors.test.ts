/// <reference types="node" />
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { validateColour, validateContrast } from './protocol'

interface StyleVector {
  readonly kind: string
  readonly name: string
  readonly expected: string
  readonly value: string
}

function parseVectors(source: string): StyleVector[] {
  return source
    .split('\n')
    .slice(1)
    .filter((line) => line.length > 0)
    .map((line) => {
      const [kind = '', name = '', expected = '', ...rest] = line.split('\t')
      return { kind, name, expected, value: rest.join('\t') }
    })
}

function outcomeOf(vector: StyleVector): string {
  if (vector.kind === 'foreground' || vector.kind === 'background') {
    return validateColour(vector.value, vector.kind) ?? 'ok'
  }
  if (vector.kind === 'contrast') {
    return validateContrast(Number(vector.value)) ?? 'ok'
  }
  throw new Error(`unknown vector kind ${vector.kind}`)
}

describe('shared style vectors', () => {
  const source = readFileSync(
    new URL('../../../crates/adapters/src/session/style_vectors.tsv', import.meta.url),
    'utf8',
  )
  const vectors = parseVectors(source)

  it('reads the same thirty-five rows the native test reads', () => {
    expect(vectors).toHaveLength(35)
  })

  it.each(vectors.map((vector) => [`${vector.kind} ${vector.name}`, vector] as const))(
    '%s validates as the native shell does',
    (_name, vector) => {
      expect(outcomeOf(vector)).toBe(vector.expected)
    },
  )
})
