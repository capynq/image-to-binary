(async () => {
  const cfg = window.__i2bE2E
  const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args)
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
  const report = (phase, body) => invoke('smoke_e2e', { report: { phase, ...body } })
  const waitFor = async (probe, what, timeout = 30000) => {
    const started = performance.now()
    for (;;) {
      const value = probe()
      if (value) return value
      if (performance.now() - started > timeout) throw new Error(`timed out waiting for ${what}`)
      await sleep(15)
    }
  }
  const header = () => document.querySelector('.header-actions')
  const headerButton = (label) =>
    [...header().querySelectorAll('button')].find((button) => button.textContent.trim() === label)
  const enabledButton = (label) =>
    [...document.querySelectorAll('button')].find(
      (button) => button.textContent.trim() === label && !button.disabled,
    )
  const clickEnabled = async (labels) => {
    const names = [].concat(labels)
    const button = await waitFor(
      () => names.map((name) => enabledButton(name)).find((found) => found !== undefined),
      `enabled ${names.join(' or ')}`,
      30000,
    )
    button.click()
  }
  const status = () => document.querySelector('.export-section .export-status')?.textContent ?? ''
  const metadata = () => document.querySelector('.metadata-list')?.textContent ?? ''
  const alerts = () => document.querySelectorAll('[role="alert"]').length
  const setValue = (element, value) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(element, String(value))
    element.dispatchEvent(new Event('input', { bubbles: true }))
  }
  const setRange = (id, value) => setValue(document.getElementById(id), value)
  const previewImage = () => {
    const image = document.querySelector('img.fixture-preview[alt="Binary artwork preview"]')
    return image !== null && image.complete && image.naturalWidth > 0 ? image : null
  }
  const pixelsOf = (image) => {
    const canvas = document.createElement('canvas')
    canvas.width = image.naturalWidth
    canvas.height = image.naturalHeight
    const context = canvas.getContext('2d')
    context.drawImage(image, 0, 0)
    return {
      width: canvas.width,
      height: canvas.height,
      rgba: Array.from(context.getImageData(0, 0, canvas.width, canvas.height).data),
    }
  }
  const openNative = async (path) => {
    await invoke('smoke_queue_open', { path })
    headerButton('Open image').click()
  }
  const startFrames = () => {
    const state = { times: [], running: true }
    const tick = (time) => {
      if (!state.running) return
      state.times.push(time)
      requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
    return state
  }
  const largestGap = (times) => {
    let gap = 0
    for (let index = 1; index < times.length; index += 1) {
      gap = Math.max(gap, times[index] - times[index - 1])
    }
    return gap
  }

  if (cfg.mode === 'drop') {
    try {
      await waitFor(() => document.querySelector('.app-shell.is-dragging'), 'native hover', 10000)
      await report('drop-hover', { hoverShown: true })
      await waitFor(
        () => metadata().includes('png23x17.png') && !document.querySelector('.app-shell.is-dragging'),
        'native drop import',
        30000,
      )
      await report('drop-accepted', { metadata: metadata() })
      await waitFor(
        () => document.querySelector('.error-text')?.textContent.startsWith('Choose one image'),
        'native rejected drop',
        30000,
      )
      await report('drop-rejected', { message: document.querySelector('.error-text').textContent })
    } catch (error) {
      await report('drop-error', { error: String(error) })
    }
    return
  }

  const out = {}
  try {
    await openNative(cfg.pngFixture)
    await waitFor(() => metadata().includes('png23x17.png'), 'PNG import')
    out.png = { metadata: metadata() }
    await clickEnabled('Render artwork')
    const preview = await waitFor(previewImage, 'PNG artwork preview')
    out.preview = pixelsOf(preview)
    await invoke('smoke_queue_save', { path: `${cfg.outDir}/png.png` })
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(() => status().startsWith('Saved '), 'PNG export to the chosen file')
    out.png.exportStatus = status()

    headerButton('Load demo').click()
    await waitFor(() => metadata().includes('Built-in demo'), 'demo import')
    out.demo = { metadata: metadata() }
    await clickEnabled('Render artwork')
    const demoPreview = await waitFor(previewImage, 'demo artwork preview')
    out.demo.preview = { width: demoPreview.naturalWidth, height: demoPreview.naturalHeight }

    await openNative(cfg.largeFixture)
    await waitFor(() => metadata().includes('large1200.png'), 'large PNG import')
    setRange('cell-size', 5)
    setRange('font-size', 10)
    await waitFor(
      () => document.querySelector('label[for="font-size"] output')?.textContent === '10px',
      'font setting',
    )
    await invoke('smoke_queue_save', { path: `${cfg.outDir}/large.png` })
    const frames = startFrames()
    const started = performance.now()
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(
      () => status().startsWith('Saved ') || status().includes('failed'),
      'large export',
      120000,
    )
    const elapsed = performance.now() - started
    frames.running = false
    out.large = {
      status: status(),
      elapsedMs: Math.round(elapsed),
      frames: frames.times.length,
      maxRafGapMs: Math.round(largestGap(frames.times)),
    }

    await invoke('smoke_queue_save', { path: `${cfg.outDir}/cancelled.png` })
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(() => enabledButton('Cancel export'), 'running export', 10000)
    enabledButton('Cancel export').click()
    await waitFor(() => status() === 'PNG export cancelled.', 'export cancellation', 60000)
    out.cancel = { status: status(), alerts: alerts() }

    await openNative(cfg.modeFixture)
    await waitFor(() => metadata().includes('mode64x40.png'), 'mode PNG import')
    setRange('cell-size', 6)
    setRange('font-size', 10)
    await waitFor(
      () => document.querySelector('label[for="font-size"] output')?.textContent === '10px',
      'mode font setting',
    )
    document.querySelector('input[type="radio"][value="ascii"]').click()
    const rampField = await waitFor(() => document.querySelector('input[name="ramp"]'), 'ASCII ramp field')
    setValue(rampField, cfg.modeRamp)
    await clickEnabled('Render artwork')
    const asciiPreview = await waitFor(previewImage, 'ASCII artwork preview')
    out.mode = {
      ramp: document.querySelector('input[name="ramp"]').value,
      preview: pixelsOf(asciiPreview),
    }
    await invoke('smoke_queue_save', { path: `${cfg.outDir}/mode-ascii.png` })
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(() => status().startsWith('Saved '), 'ASCII export to the chosen file')
    out.mode.exportStatus = status()
    document.querySelector('input[type="radio"][value="binary"]').click()
    await waitFor(() => document.querySelector('input[name="ramp"]') === null, 'classic mode restored')

    headerButton('Reset').click()
    await waitFor(() => document.querySelector('.metadata-list') === null, 'reset')
    out.reset = { metadataCleared: true, emptyCanvas: document.querySelector('.empty-canvas') !== null }

    await openNative(null)
    await sleep(400)
    out.openCancel = { alerts: alerts(), metadataAbsent: document.querySelector('.metadata-list') === null }

    await openNative(cfg.pngFixture)
    await waitFor(() => metadata().includes('png23x17.png'), 'PNG import before save cancel')
    await invoke('smoke_queue_save', { path: null })
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(() => status() === 'PNG export cancelled.', 'save dialog cancellation', 20000)
    out.saveCancel = { status: status(), alerts: alerts(), errorPanel: document.querySelector('.export-error') !== null }
    headerButton('Reset').click()
    await waitFor(() => document.querySelector('.metadata-list') === null, 'final reset')
    out.cspViolations = window.__imageToBinarySmoke?.violations ?? -1
    out.loads = [
      ...performance.getEntriesByType('resource').map((entry) => entry.name),
      ...(window.__imageToBinarySmoke?.workers ?? []),
    ].filter((name) => /worker|\.wasm/i.test(name))
    await report('main', out)
  } catch (error) {
    await report('main-error', { ...out, error: String(error) })
  }
})()
