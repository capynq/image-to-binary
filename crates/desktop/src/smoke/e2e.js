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
  const artworkSrc = () =>
    document.querySelector('img.fixture-preview[alt="Binary artwork preview"]')?.getAttribute('src') ?? null
  const artworkIdle = () => document.querySelector('.artifact-frame.is-refreshing') === null
  const newArtwork = (previous, what) =>
    waitFor(() => {
      const image = previewImage()
      return image !== null && image.getAttribute('src') !== previous && artworkIdle() ? image : null
    }, what, 60000)
  const settledArtwork = async () => {
    let seen = null
    let quietSince = performance.now()
    await waitFor(() => {
      const current = artworkSrc()
      if (previewImage() === null || !artworkIdle() || current !== seen) {
        seen = current
        quietSince = performance.now()
        return null
      }
      return performance.now() - quietSince >= 400 ? true : null
    }, 'settled artwork', 60000)
    return artworkSrc()
  }
  const awaitAutoArtwork = () =>
    waitFor(() => metadata().includes('Artwork preview'), 'automatic artwork after import', 60000)
  const showArtwork = () =>
    [...document.querySelectorAll('.view-toggle label')]
      .find((label) => label.textContent.trim() === 'Artwork')
      .click()
  const badgeAfter = async (change) => {
    change()
    const started = performance.now()
    for (;;) {
      if (document.querySelector('.updating-badge') !== null) return true
      if (performance.now() - started > 200) return false
      await sleep(5)
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
    await awaitAutoArtwork()
    showArtwork()
    const preview = await newArtwork(null, 'PNG artwork shown after import')
    out.preview = pixelsOf(preview)
    out.autoRender = { importRendered: true }
    await invoke('smoke_queue_save', { path: `${cfg.outDir}/png.png` })
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(() => status().startsWith('Saved '), 'PNG export to the chosen file')
    out.png.exportStatus = status()

    headerButton('Load demo').click()
    await waitFor(() => metadata().includes('Built-in demo'), 'demo import')
    out.demo = { metadata: metadata() }
    await awaitAutoArtwork()
    showArtwork()
    const demoPreview = await newArtwork(null, 'demo artwork shown after import')
    out.demo.preview = { width: demoPreview.naturalWidth, height: demoPreview.naturalHeight }

    await openNative(cfg.largeFixture)
    await waitFor(() => metadata().includes('large1200.png'), 'large PNG import')
    await awaitAutoArtwork()
    showArtwork()
    await newArtwork(null, 'large artwork shown after import')
    const beforeResize = await settledArtwork()
    const resizeBadge = await badgeAfter(() => {
      setRange('cell-size', 5)
      setRange('font-size', 10)
    })
    await waitFor(
      () => document.querySelector('output[for="font-size"]')?.textContent === '10px',
      'font setting',
    )
    await newArtwork(beforeResize, 'large artwork rendered after cell size change')
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
    await awaitAutoArtwork()
    showArtwork()
    await newArtwork(null, 'mode artwork shown after import')
    const beforeAscii = await settledArtwork()
    setRange('cell-size', 6)
    setRange('font-size', 10)
    await waitFor(
      () => document.querySelector('output[for="font-size"]')?.textContent === '10px',
      'mode font setting',
    )
    document.querySelector('input[type="radio"][value="ascii"]').click()
    const rampField = await waitFor(() => document.querySelector('input[name="ramp"]'), 'ASCII ramp field')
    const rampBadge = await badgeAfter(() => setValue(rampField, cfg.modeRamp))
    const asciiPreview = await newArtwork(beforeAscii, 'ASCII artwork rendered after ramp change')
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

    const styleBefore = await settledArtwork()
    const foregroundField = await waitFor(() => document.querySelector('input[name="foreground"]'), 'foreground colour field')
    const colourBadge = await badgeAfter(() => {
      setValue(foregroundField, '#ff8000')
      setValue(document.querySelector('input[name="background"]'), '#102040')
    })
    await waitFor(
      () => document.querySelector('input[name="background"]').value === '#102040' && foregroundField.value === '#ff8000',
      'colours applied',
    )
    await newArtwork(styleBefore, 'styled artwork rendered after colour change')
    const beforeManual = artworkSrc()
    await clickEnabled('Render artwork')
    const styledPreview = await newArtwork(beforeManual, 'styled artwork after the manual Render')
    out.autoRender.badges = { cellSize: resizeBadge, ramp: rampBadge, colour: colourBadge }
    out.style = {
      foreground: foregroundField.value,
      background: document.querySelector('input[name="background"]').value,
      colorMode: document.querySelector('input[type="radio"][value="monochrome"]').checked ? 'monochrome' : 'other',
      preview: pixelsOf(styledPreview),
    }
    await invoke('smoke_queue_save', { path: `${cfg.outDir}/style-mono.png` })
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(() => status().startsWith('Saved '), 'styled export to the chosen file')
    out.style.exportStatus = status()

    const formatRadio = (value) => document.querySelector(`input[type="radio"][value="${value}"]`)
    const exportText = async (file) => {
      await invoke('smoke_queue_save', { path: `${cfg.outDir}/${file}` })
      await clickEnabled(['Export text', 'Export again'])
      await waitFor(() => status().startsWith(`Saved ${file}`), `text export to ${file}`)
      return status()
    }
    formatRadio('text').click()
    await waitFor(() => formatRadio('text').checked, 'text format chosen')
    out.text = {}
    out.text.classic = await exportText('classic.txt')
    document.querySelector('input[type="radio"][value="ascii"]').click()
    const textRamp = await waitFor(() => document.querySelector('input[name="ramp"]'), 'ASCII ramp field for text')
    setValue(textRamp, cfg.modeRamp)
    await waitFor(() => document.querySelector('input[name="ramp"]').value === cfg.modeRamp, 'text ramp applied')
    out.text.asciiCustom = await exportText('ascii-custom.txt')
    document.querySelector('input[type="radio"][value="block"]').click()
    await waitFor(() => document.querySelector('input[type="radio"][value="block"]').checked, 'block mode for text')
    out.text.block = await exportText('block.txt')
    document.querySelector('input[type="radio"][value="ascii"]').click()
    const contrastRamp = await waitFor(() => document.querySelector('input[name="ramp"]'), 'ASCII ramp field for contrast text')
    setValue(contrastRamp, ' .:-=+*#%@')
    await waitFor(() => document.querySelector('input[name="ramp"]').value === ' .:-=+*#%@', 'default ramp for contrast text')
    setValue(document.querySelector('input[name="contrast"]'), 100)
    await waitFor(() => Number(document.querySelector('input[name="contrast"]').value) === 100, 'contrast raised')
    out.text.contrastRaised = await exportText('contrast-raised.txt')
    setValue(document.querySelector('input[name="contrast"]'), 0)
    await waitFor(() => Number(document.querySelector('input[name="contrast"]').value) === 0, 'contrast reset')
    formatRadio('png').click()
    await waitFor(() => formatRadio('png').checked, 'PNG format restored')
    document.querySelector('input[type="radio"][value="binary"]').click()
    await waitFor(() => document.querySelector('input[name="ramp"]') === null, 'classic mode restored after text')

    headerButton('Reset').click()
    await waitFor(() => document.querySelector('.metadata-list') === null, 'reset')
    out.reset = { metadataCleared: true, emptyCanvas: document.querySelector('.empty-canvas') !== null }

    await openNative(null)
    await sleep(400)
    out.openCancel = { alerts: alerts(), metadataAbsent: document.querySelector('.metadata-list') === null }

    await openNative(cfg.pngFixture)
    await waitFor(() => metadata().includes('png23x17.png'), 'PNG import before save cancel')
    await awaitAutoArtwork()
    await invoke('smoke_queue_save', { path: null })
    await clickEnabled(['Export PNG', 'Export again'])
    await waitFor(() => status() === 'PNG export cancelled.', 'save dialog cancellation', 20000)
    out.saveCancel = { status: status(), alerts: alerts(), errorPanel: document.querySelector('.export-error') !== null }
    headerButton('Reset').click()
    await waitFor(() => document.querySelector('.metadata-list') === null, 'final reset')
    await openNative(cfg.pngFixture)
    await waitFor(() => metadata().includes('png23x17.png'), 'PNG import for menu checks')
    await awaitAutoArtwork()
    showArtwork()
    await newArtwork(null, 'PNG artwork shown for menu checks')
    const settle = () => sleep(250)
    const activeLabel = () => (document.activeElement?.textContent ?? '').trim()
    const selectedView = () => document.querySelector('.view-toggle label.is-selected')?.textContent.trim() ?? ''
    const menu = {}
    for (let attempt = 0; attempt < 100; attempt += 1) {
      menu.imported = await invoke('smoke_menu_snapshot')
      if (menu.imported['file.export'] === true) break
      await sleep(100)
    }
    document.querySelector('input[type="radio"][value="ascii"]').click()
    const menuRamp = await waitFor(() => document.querySelector('input[name="ramp"]'), 'ASCII ramp for menu checks')
    setValue(menuRamp, 'café')
    await waitFor(() => menuRamp.getAttribute('aria-invalid') === 'true', 'invalid ramp for menu checks')
    await settle()
    menu.draftSnapshot = await invoke('smoke_menu_snapshot')
    const blockedStatus = status()
    const blockedPreview = document.querySelector('img.fixture-preview')?.getAttribute('src') ?? null
    await invoke('smoke_queue_save', { path: null })
    menu.blockedDispatched = [
      await invoke('smoke_menu_action', { id: 'file.export' }),
      await invoke('smoke_menu_action', { id: 'view.render' }),
    ]
    await settle()
    menu.blocked = {
      statusUnchanged: status() === blockedStatus,
      previewUnchanged: (document.querySelector('img.fixture-preview')?.getAttribute('src') ?? null) === blockedPreview,
    }
    setValue(menuRamp, ' .:-=+*#%@')
    await waitFor(() => menuRamp.getAttribute('aria-invalid') === 'false', 'valid ramp restored for menu checks')
    await settle()
    menu.validSnapshot = await invoke('smoke_menu_snapshot')
    document.querySelector('input[type="radio"][value="binary"]').click()
    await waitFor(() => document.querySelector('input[name="ramp"]') === null, 'classic mode for menu checks')
    document.activeElement?.blur()
    await invoke('smoke_menu_action', { id: 'file.export' })
    await waitFor(() => status() === 'PNG export cancelled.', 'menu export cancelled by the save dialog', 20000)
    await settle()
    menu.cancelledSave = { status: status(), focus: activeLabel() }
    await invoke('smoke_menu_action', { id: 'view.original' })
    await waitFor(() => selectedView() === 'Original', 'menu original view')
    menu.original = selectedView()
    await invoke('smoke_menu_action', { id: 'view.artwork' })
    await waitFor(() => selectedView() === 'Artwork', 'menu artwork view')
    menu.artwork = selectedView()
    await invoke('smoke_menu_action', { id: 'view.split' })
    await waitFor(() => selectedView() === 'Split' && document.querySelector('[role="slider"]') !== null, 'menu split view')
    await settle()
    menu.split = {
      view: selectedView(),
      divider: document.querySelector('[role="slider"]')?.getAttribute('aria-valuenow') ?? null,
      images: document.querySelectorAll('.comparison-clip img').length,
      focusIsBody: document.activeElement === document.body || document.activeElement === null,
    }
    const beforeMenuRender = await settledArtwork()
    await invoke('smoke_menu_action', { id: 'view.render' })
    await newArtwork(beforeMenuRender, 'menu render preview')
    menu.rendered = true
    await invoke('smoke_queue_open', { path: null })
    await invoke('smoke_menu_action', { id: 'file.open' })
    await settle()
    menu.openCancel = {
      focusIsBody: document.activeElement === document.body || document.activeElement === null,
      focus: activeLabel(),
      metadataKept: metadata().includes('png23x17.png'),
      alerts: alerts(),
    }
    const idleStatus = status()
    await invoke('smoke_menu_action', { id: 'view.cancel' })
    await settle()
    menu.cancelIdle = { unchanged: status() === idleStatus, snapshot: (await invoke('smoke_menu_snapshot'))['view.cancel'] }
    await invoke('smoke_queue_save', { path: `${cfg.outDir}/menu.png` })
    document.activeElement?.blur()
    await invoke('smoke_menu_action', { id: 'file.export' })
    await waitFor(() => status().startsWith('Saved '), 'menu PNG export to the chosen file')
    await settle()
    menu.savedExport = { status: status(), focus: activeLabel() }
    await openNative(cfg.largeFixture)
    await waitFor(() => metadata().includes('large1200.png'), 'large import for menu cancel')
    await awaitAutoArtwork()
    await waitFor(() => enabledButton('Export PNG') ?? enabledButton('Export again'), 'large import ready to export', 60000)
    await invoke('smoke_queue_save', { path: `${cfg.outDir}/menu-cancelled.png` })
    document.activeElement?.blur()
    await invoke('smoke_menu_action', { id: 'file.export' })
    await waitFor(() => enabledButton('Cancel export'), 'running export before menu cancel', 10000)
    const running = await invoke('smoke_menu_snapshot')
    await invoke('smoke_menu_action', { id: 'view.cancel' })
    await waitFor(() => status() === 'PNG export cancelled.', 'menu cancel of a running export', 60000)
    await settle()
    menu.cancelRunning = {
      status: status(),
      exportDisabled: running['file.export'] === false,
      cancelEnabled: running['view.cancel'] === true,
      focus: activeLabel(),
    }
    headerButton('Reset').click()
    await waitFor(() => document.querySelector('.metadata-list') === null, 'menu checks reset')
    menu.resetFocus = activeLabel()
    out.menu = menu
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
