import { describe, expect, it } from 'vitest'
import { EXPORT_MIME_TYPES } from '../processing/types'
import { EXPORT_FORMAT_WORDS, EXPORT_FORMATS, exportFormatChoices } from './exportFormats'

describe('exportFormatChoices', () => {
  it('offers PNG and text for a still and adds GIF only for an animation', () => {
    expect(exportFormatChoices(false)).toEqual(['png', 'text'])
    expect(exportFormatChoices(true)).toEqual(['png', 'text', 'gif'])
    expect(EXPORT_FORMATS).toEqual(['png', 'text', 'gif'])
  })

  it('keeps the list order the choice control shows', () => {
    expect(exportFormatChoices(true)).toEqual([...EXPORT_FORMATS])
  })
})

describe('GIF format words and type', () => {
  it('labels GIF as an animated export that stays on this device', () => {
    expect(EXPORT_FORMAT_WORDS.gif.choice).toBe('Animated GIF')
    expect(EXPORT_FORMAT_WORDS.gif.name).toBe('GIF')
    expect(EXPORT_FORMAT_WORDS.gif.help).toContain('every frame')
    expect(EXPORT_FORMAT_WORDS.gif.help).toContain('stays on this device')
  })

  it('uses the image/gif Blob type', () => {
    expect(EXPORT_MIME_TYPES.gif).toBe('image/gif')
  })
})
