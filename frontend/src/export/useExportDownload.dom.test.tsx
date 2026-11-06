/** @vitest-environment jsdom */
import { StrictMode, act } from 'react'
import type { ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import type { Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { ExportArtifact } from '../processing/types'
import { artifactOf, countingUrlApi } from './exportTestSupport'
import type { CountingUrlApi } from './exportTestSupport'
import { useExportDownload } from './useExportDownload'

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })

function Harness(props: {
  readonly artifact: ExportArtifact | null
  readonly urlApi: CountingUrlApi
}) {
  const link = useExportDownload(props.artifact, 'a-binary.png', props.urlApi)
  return <output>{link?.url ?? 'none'}</output>
}

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  unmount()
  container.remove()
})

function unmount(): void {
  if (root === null) return
  const mounted = root
  root = null
  act(() => mounted.unmount())
}

function render(element: ReactElement): void {
  if (root === null) root = createRoot(container)
  const target = root
  act(() => target.render(element))
}

describe('useExportDownload lifecycle', () => {
  it('revokes the object URL when the component unmounts', () => {
    const urls = countingUrlApi()
    const artifact = artifactOf(16)
    render(<Harness artifact={artifact} urlApi={urls} />)
    const url = container.textContent ?? ''
    expect(url).toBe('blob:export/1')
    expect(urls.live.has(url)).toBe(true)

    unmount()

    expect(urls.revoked).toEqual([url])
    expect(urls.live.size).toBe(0)
  })

  it('revokes the previous URL when the artifact is replaced', () => {
    const urls = countingUrlApi()
    render(<Harness artifact={artifactOf(16)} urlApi={urls} />)
    render(<Harness artifact={artifactOf(32)} urlApi={urls} />)
    expect(urls.revoked).toEqual(['blob:export/1'])
    expect([...urls.live]).toEqual(['blob:export/2'])
    expect(container.textContent).toBe('blob:export/2')
  })

  it('leaves exactly one live URL after StrictMode double effects', () => {
    const urls = countingUrlApi()
    render(
      <StrictMode>
        <Harness artifact={artifactOf(16)} urlApi={urls} />
      </StrictMode>,
    )
    expect(urls.created).toBe(2)
    expect(urls.revoked).toEqual(['blob:export/1'])
    expect(urls.live.size).toBe(1)
    expect(container.textContent).toBe([...urls.live][0])
  })
})
