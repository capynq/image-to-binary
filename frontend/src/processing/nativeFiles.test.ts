import { describe, expect, it } from 'vitest'
import type { StudioController } from '../state/controller'
import { readDropOutcome, readFileReply, importNativeFile } from './nativeFiles'
import type { NativeFileInput, StudioSnapshot } from './types'
import { ProcessingWorkerError } from './workerClient'

describe('native file replies', () => {
  it('accepts a well-formed file reply and rejects any other shape', () => {
    expect(readFileReply({ id: 3, name: 'a.png', byteLength: 10 })).toEqual({
      id: 3,
      name: 'a.png',
      byteLength: 10,
    })
    for (const value of [
      null,
      'x',
      { id: -1, name: 'a', byteLength: 1 },
      { id: 1, name: 2, byteLength: 1 },
      { id: 1, name: 'a', byteLength: 1.5 },
    ]) {
      expect(() => readFileReply(value)).toThrow(ProcessingWorkerError)
    }
  })

  it('turns a malformed drop payload into a rejected drop with a readable message', () => {
    expect(readDropOutcome({ status: 'accepted', file: { id: 'x' } })).toEqual({
      status: 'rejected',
      error: { code: 'internal', message: 'The native shell sent a malformed reply.' },
    })
    expect(readDropOutcome(42)).toMatchObject({ status: 'rejected' })
  })

  it('keeps a readable rejection exactly as the shell wrote it', () => {
    expect(
      readDropOutcome({
        status: 'rejected',
        error: { code: 'invalid_argument', message: 'Choose one image at a time.' },
      }),
    ).toEqual({
      status: 'rejected',
      error: { code: 'invalid_argument', message: 'Choose one image at a time.' },
    })
  })
})

describe('importNativeFile', () => {
  const file: NativeFileInput = { kind: 'native-file', id: 12, name: 'a.png', byteLength: 4 }

  function fakes(status: StudioSnapshot['status']) {
    const released: number[] = []
    const imported: unknown[] = []
    const controller = {
      getSnapshot: () => ({ status }) as StudioSnapshot,
      import: async (input: NativeFileInput) => {
        imported.push(input)
      },
    }
    const files = {
      release: async (id: number) => {
        released.push(id)
      },
    }
    return { controller, files, released, imported }
  }

  it('releases the handle without importing when the controller is already disposed', async () => {
    const { controller, files, released, imported } = fakes('disposed')
    await importNativeFile(
      controller as Pick<StudioController, 'getSnapshot' | 'import'>,
      files,
      file,
    )
    expect(released).toEqual([12])
    expect(imported).toEqual([])
  })

  it('leaves the handle to the adapter when the import is started', async () => {
    const { controller, files, released, imported } = fakes('ready')
    await importNativeFile(
      controller as Pick<StudioController, 'getSnapshot' | 'import'>,
      files,
      file,
    )
    expect(imported).toEqual([file])
    expect(released).toEqual([])
  })
})
