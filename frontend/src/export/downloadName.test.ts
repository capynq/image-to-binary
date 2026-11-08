import { describe, expect, it } from 'vitest'
import { exportFilename } from './downloadName'

const encoder = new TextEncoder()

describe('exportFilename', () => {
  it('derives the stem from the accepted source name', () => {
    expect(exportFilename('photo.png')).toBe('photo-binary.png')
    expect(exportFilename('my.photo.v2.jpeg')).toBe('my.photo.v2-binary.png')
    expect(exportFilename('archive')).toBe('archive-binary.png')
  })

  it('falls back to a fixed stem when nothing usable remains', () => {
    expect(exportFilename('')).toBe('image-binary.png')
    expect(exportFilename('.png')).toBe('image-binary.png')
    expect(exportFilename('...')).toBe('image-binary.png')
  })

  it('replaces path separators and drive colons so the name is a single segment', () => {
    expect(exportFilename('../../etc/passwd.png')).toBe('_._etc_passwd-binary.png')
    const windows = exportFilename('C:\\Users\\me\\pic.bmp')
    expect(windows).toBe('C__Users_me_pic-binary.png')
    expect(windows).not.toMatch(/[\\/:]/)
  })

  it('replaces control and bidirectional override characters', () => {
    expect(exportFilename('a\u0000b\u0007c.png')).toBe('a_b_c-binary.png')
    expect(exportFilename('abc\u202edef.png')).toBe('abc_def-binary.png')
  })

  it('collapses whitespace and trims dots at both ends', () => {
    expect(exportFilename('  my   pic  .png')).toBe('my pic-binary.png')
    expect(exportFilename('name....png')).toBe('name-binary.png')
  })

  it('prefixes Windows reserved device names only', () => {
    expect(exportFilename('CON.png')).toBe('_CON-binary.png')
    expect(exportFilename('lpt1.bmp')).toBe('_lpt1-binary.png')
    expect(exportFilename('console.png')).toBe('console-binary.png')
  })

  it('treats the first dot-delimited segment of the stem as the device name', () => {
    expect(exportFilename('nul.backup.png')).toBe('_nul.backup-binary.png')
    expect(exportFilename('con.old.png')).toBe('_con.old-binary.png')
    expect(exportFilename('COM1.5.png')).toBe('_COM1.5-binary.png')
    expect(exportFilename('my.nul.png')).toBe('my.nul-binary.png')
  })

  it('strips zero-width, byte-order and bidirectional format characters', () => {
    expect(exportFilename('a​b.png')).toBe('a_b-binary.png')
    expect(exportFilename('a‎b.png')).toBe('a_b-binary.png')
    expect(exportFilename('a﻿b.png')).toBe('a_b-binary.png')
    expect(exportFilename('a⁦b⁩c.png')).toBe('a_b_c-binary.png')
    expect(exportFilename('a‪b‬c.png')).toBe('a_b_c-binary.png')
  })

  it('neutralises a right-to-left override that would spoof the extension', () => {
    const spoofed = exportFilename('report‮gnp.exe')
    expect(spoofed).toBe('report_gnp-binary.png')
    expect(spoofed).not.toMatch(/\p{Cf}/u)
    expect(exportFilename('evil‮cod.png')).toBe('evil_cod-binary.png')
  })

  it('bounds very long ASCII names to 160 UTF-8 bytes', () => {
    const name = exportFilename(`${'x'.repeat(400)}.png`)
    expect(name).toBe(`${'x'.repeat(160)}-binary.png`)
    expect(encoder.encode(name).byteLength).toBeLessThanOrEqual(255)
  })

  it('keeps multi-byte characters whole when bounding Unicode names', () => {
    const cjk = exportFilename(`${'猫'.repeat(200)}.jpg`)
    expect(cjk).toBe(`${'猫'.repeat(53)}-binary.png`)
    expect(encoder.encode(cjk).byteLength).toBeLessThanOrEqual(255)

    const emoji = exportFilename(`${'🙂'.repeat(100)}.png`)
    expect(emoji).toBe(`${'🙂'.repeat(40)}-binary.png`)
    expect(emoji).not.toContain('\uFFFD')
  })
})

describe('exportFilename for text exports', () => {
  it('keeps PNG as the default format and yields the same names as before', () => {
    expect(exportFilename('photo.png', 'png')).toBe('photo-binary.png')
    expect(exportFilename('photo.png')).toBe(exportFilename('photo.png', 'png'))
  })

  it('names a text export <stem>-binary.txt with the same stem rules as PNG', () => {
    expect(exportFilename('photo.png', 'text')).toBe('photo-binary.txt')
    expect(exportFilename('my.photo.v2.jpeg', 'text')).toBe('my.photo.v2-binary.txt')
    expect(exportFilename('archive', 'text')).toBe('archive-binary.txt')
    expect(exportFilename('', 'text')).toBe('image-binary.txt')
  })

  it('guards Windows device names and bounds the stem for text too', () => {
    expect(exportFilename('nul.backup', 'text')).toBe('_nul-binary.txt')
    const long = exportFilename(`${'é'.repeat(200)}.png`, 'text')
    expect(long.endsWith('-binary.txt')).toBe(true)
    expect(encoder.encode(long.slice(0, -'-binary.txt'.length)).byteLength).toBeLessThanOrEqual(160)
  })
})

describe('exportFilename for GIF exports', () => {
  it('names a GIF export <stem>-binary.gif', () => {
    expect(exportFilename('wave.gif', 'gif')).toBe('wave-binary.gif')
    expect(exportFilename('', 'gif')).toBe('image-binary.gif')
  })
})
