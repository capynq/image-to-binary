import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { createDemoProcessingAdapter } from '../processing/demoAdapter'
import type { BlobExportArtifact, UrlApi } from '../processing/types'
import { createStudioController } from '../state/controller'
import { createDownloadOwner } from './downloadOwner'
import type { DownloadLink } from './downloadOwner'
import { ExportPanel } from './ExportPanel'
import type { ExportPanelProps } from './ExportPanel'

const noop = () => undefined

function baseProps(overrides: Partial<ExportPanelProps> = {}): ExportPanelProps {
  return {
    phase: 'idle',
    canExport: true,
    disabledReason: null,
    progress: 0,
    byteLength: null,
    error: null,
    download: null,
    onExport: noop,
    onCancel: noop,
    ...overrides,
  }
}

function render(overrides: Partial<ExportPanelProps>): string {
  return renderToStaticMarkup(<ExportPanel {...baseProps(overrides)} />)
}

function countingUrlApi(): UrlApi & { created: number; revoked: string[] } {
  const state = { created: 0, revoked: [] as string[] }
  return {
    get created() {
      return state.created
    },
    revoked: state.revoked,
    createObjectURL() {
      state.created += 1
      return `blob:export/${state.created}`
    },
    revokeObjectURL(url) {
      state.revoked.push(url)
    },
  }
}

function linkFor(size: number, filename: string, urls: UrlApi): DownloadLink {
  const artifact: BlobExportArtifact = {
    kind: 'blob',
    format: 'png',
    blob: new Blob([new Uint8Array(size)], { type: 'image/png' }),
    byteLength: size,
    dispose: () => undefined,
  }
  const link = createDownloadOwner(urls).show(artifact, filename)
  if (link === null) throw new Error('A link was expected for a non-null artifact.')
  return link
}

describe('ExportPanel states', () => {
  it('idle: offers an enabled Export PNG action without a reason', () => {
    const html = render({})
    expect(html).toContain('Export PNG')
    expect(html).not.toMatch(/<button[^>]*disabled=""[^>]*>Export PNG/)
    expect(html).not.toContain('aria-describedby')
    expect(html).not.toContain('Download PNG')
    expect(html).not.toContain('<progress')
  })

  it('idle without a source: disables Export and shows the reason it is linked to', () => {
    const html = render({
      canExport: false,
      disabledReason: 'Open or load an image to enable PNG export.',
    })
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Export PNG/)
    expect(html).toContain('Open or load an image to enable PNG export.')
    expect(html).toMatch(/aria-describedby="([^"]+)"/)
    const id = /aria-describedby="([^"]+)"/.exec(html)?.[1] ?? ''
    expect(html).toContain(`id="${id}"`)
  })

  it('running: shows a labelled progressbar with percentage text and a Cancel control', () => {
    const html = render({ phase: 'running', progress: 0.42 })
    expect(html).toContain('<progress')
    expect(html).toContain('max="100"')
    expect(html).toContain('value="42"')
    expect(html).toMatch(/<span class="export-percent">42%<\/span>/)
    expect(html).toMatch(/<label for="[^"]+">Rendering PNG<\/label>/)
    expect(html).toContain('Cancel export')
    expect(html).not.toContain('Export PNG')
    expect(html).toContain('Rendering PNG export.')
  })

  it('running: clamps out-of-range progress into 0 to 100 percent', () => {
    expect(render({ phase: 'running', progress: 1.7 })).toContain('value="100"')
    expect(render({ phase: 'running', progress: -1 })).toContain('value="0"')
  })

  it('preparing: shows the completed progress without a Cancel control', () => {
    const html = render({ phase: 'preparing', progress: 1 })
    expect(html).toContain('value="100"')
    expect(html).toContain('Preparing download')
    expect(html).not.toContain('Cancel export')
    expect(html).toContain('Preparing PNG download.')
  })

  it('ready: offers the Download link with its filename and the encoded size', () => {
    const urls = countingUrlApi()
    const download = linkFor(2048, 'photo-binary.png', urls)
    const html = render({ phase: 'ready', byteLength: 2048, download })
    expect(html).toContain('Encoded PNG: 2.0 KiB')
    expect(html).toContain('href="blob:export/1"')
    expect(html).toContain('download="photo-binary.png"')
    expect(html).toContain('Download PNG')
    expect(html).toContain('Export again')
    expect(html).toContain('PNG export ready, 2.0 KiB.')
    expect(urls.revoked).toHaveLength(0)
  })

  it('ready without a link renders no Download control', () => {
    const html = render({ phase: 'ready', byteLength: 2048, download: null })
    expect(html).not.toContain('Download PNG')
    expect(html).not.toContain('href=')
  })

  it('failed: shows the full error text and a Retry control, never a download', () => {
    const message =
      'The PNG encoder stopped after 3 of 9 chunks. The encoded output would exceed the browser ' +
      'limit, so the export was not produced. Reduce the cell size or choose a smaller image.'
    const html = render({ phase: 'failed', error: message })
    expect(html).toContain(message)
    expect(html).toContain('Retry PNG export')
    expect(html).toContain('PNG export failed. ' + message)
    expect(html).not.toContain('Download PNG')
    expect(html).not.toContain('Export PNG')
  })

  it('live region is the single polite status region in the panel', () => {
    const html = render({ phase: 'failed', error: 'Boom.' })
    expect(html.match(/role="status"/g)).toHaveLength(1)
    expect(html).not.toContain('role="alert"')
  })

  it('exposes the heading as the fallback focus target', () => {
    expect(render({})).toContain('tabindex="-1"')
  })
})

describe('reset removes exportable state', () => {
  it('reset removes the Download control and revokes its object URL', async () => {
    const urls = countingUrlApi()
    const adapter = createDemoProcessingAdapter()
    const pngAsText: typeof adapter = {
      ...adapter,
      export: (session, source, _format, settings, context) =>
        adapter.export(session, source, 'text', settings, context),
    }
    const controller = createStudioController({ adapter: pngAsText })
    await controller.loadDemo()
    await controller.startExport()
    const artifact = controller.readyExport() as BlobExportArtifact | null
    expect(artifact).not.toBeNull()

    const owner = createDownloadOwner(urls)
    const download = owner.show(artifact, 'demo-binary.png')
    expect(download).not.toBeNull()
    expect(urls.created).toBe(1)
    expect(render({ phase: 'ready', byteLength: 10, download })).toContain('Download PNG')

    controller.reset()
    expect(controller.readyExport()).toBeNull()
    expect(controller.getSnapshot().export.status).toBe('idle')
    const afterReset = owner.show(
      controller.readyExport() as BlobExportArtifact | null,
      'image-binary.png',
    )
    expect(afterReset).toBeNull()
    expect(urls.revoked).toEqual(['blob:export/1'])
    expect(render({ phase: 'idle', download: afterReset })).not.toContain('Download PNG')
    owner.dispose()
    expect(urls.revoked).toHaveLength(1)
  })
})
