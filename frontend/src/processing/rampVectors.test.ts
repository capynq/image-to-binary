/// <reference types="node" />
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { validateRamp } from './protocol'

interface RampVector {
  readonly name: string
  readonly expected: string
  readonly ramp: string
}

function decodeEscapes(escaped: string): string {
  return escaped.replace(/\\u([0-9A-Fa-f]{4})/g, (_, hex: string) =>
    String.fromCharCode(Number.parseInt(hex, 16)),
  )
}

function parseVectors(source: string): RampVector[] {
  return source
    .split('\n')
    .slice(1)
    .filter((line) => line.length > 0)
    .map((line) => {
      const [name = '', expected = '', ...rest] = line.split('\t')
      return { name, expected, ramp: decodeEscapes(rest.join('\t')) }
    })
}

describe('shared ramp vectors', () => {
  const source = readFileSync(
    new URL('../../../crates/adapters/src/session/ramp_vectors.tsv', import.meta.url),
    'utf8',
  )
  const vectors = parseVectors(source)

  it('reads the same fourteen rows the native test reads', () => {
    expect(vectors).toHaveLength(14)
  })

  it.each(vectors.map((vector) => [vector.name, vector] as const))(
    '%s validates as the native shell does',
    (_name, vector) => {
      const outcome = validateRamp(vector.ramp)
      expect(outcome ?? 'ok').toBe(vector.expected)
    },
  )
})
