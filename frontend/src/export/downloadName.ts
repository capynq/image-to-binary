import type { ExportFormat } from '../processing/types'
const MAX_STEM_BYTES = 160
const EXTENSION = /\.[^.]*$/
/* Format characters (\p{Cf}) cover zero-width marks, U+FEFF and the bidi embedding, override and isolate controls that can spoof an extension. */
const UNSAFE_CHARACTERS = /[\\/:*?"<>|\u0000-\u001f\u007f-\u009f\p{Cf}]/gu
const WINDOWS_RESERVED = /^(con|prn|aux|nul|com[0-9]|lpt[0-9])$/i
const encoder = new TextEncoder()

const EXTENSIONS: Readonly<Record<ExportFormat, string>> = Object.freeze({
  png: 'png',
  text: 'txt',
  gif: 'gif',
})

/**
 * Derives the download name of an export of `format` from the accepted source name: the last
 * extension is removed, path separators, control and format characters become underscores, and
 * the stem is bounded to 160 UTF-8 bytes on code point boundaries. Empty stems get a fixed name.
 * A stem whose first dot-delimited segment is a Windows device name (`nul.backup`) is prefixed
 * with an underscore. PNG is the default format and yields `<stem>-binary.png`; text yields
 * `<stem>-binary.txt`.
 */
export function exportFilename(sourceName: string, format: ExportFormat = 'png'): string {
  const stem = sourceName
    .replace(EXTENSION, '')
    .replace(UNSAFE_CHARACTERS, '_')
    .replace(/\.{2,}/g, '.')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^[.\s]+|[.\s]+$/g, '')
  const bounded = truncateUtf8(stem, MAX_STEM_BYTES).replace(/[.\s]+$/g, '')
  const firstSegment = bounded.split('.')[0] ?? ''
  const named =
    bounded === '' ? 'image' : WINDOWS_RESERVED.test(firstSegment) ? `_${bounded}` : bounded
  return `${named}-binary.${EXTENSIONS[format]}`
}

function truncateUtf8(value: string, maxBytes: number): string {
  let result = ''
  let bytes = 0
  for (const character of value) {
    const size = encoder.encode(character).byteLength
    if (bytes + size > maxBytes) break
    result += character
    bytes += size
  }
  return result
}
