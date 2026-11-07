import assert from 'node:assert/strict';
import { test } from 'node:test';

import { bytes, ImageSession, JobStage, loadReference, MAX_SAFE } from './support.mjs';

const reference = await loadReference();
const PNG_SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10];
const UTF8_BOM = [0xef, 0xbb, 0xbf];
const decoder = new TextDecoder('utf-8', { fatal: true });

function concat(chunks) {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

/** Starts an export of `outputKind` with the fixture's classic grid and the given threshold. */
function startExport(session, generation, fixture, threshold, outputKind) {
  return session.startExport(
    generation,
    fixture.cellSize,
    fixture.fontSize,
    threshold,
    fixture.sampling,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    undefined,
    outputKind,
  );
}

/** Advances an export to Ready with `budget`, draining after every advance, then finishes it. */
function runExport(session, generation, budget) {
  const drained = [];
  while (session.stage() !== JobStage.Ready) {
    session.advance(generation, budget);
    drained.push(...session.drainExport(generation));
  }
  return concat([...drained, ...session.finishExport(generation)]);
}

function textOf(session, fixture, variant, budget) {
  const stage = startExport(session, 0, fixture, variant.threshold, 'text');
  assert.equal(stage, JobStage.Sampling);
  return runExport(session, 0, budget);
}

function pngHeaderSize(png) {
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
  return { width: view.getUint32(16), height: view.getUint32(20) };
}

test('text exports keep the contract and the PNG grid for a one-byte and a large budget', () => {
  for (const fixture of reference.cases) {
    for (const variant of fixture.variants) {
      const label = `${fixture.name}@${variant.threshold}`;
      const source = bytes(fixture.source);
      const pngSession = ImageSession.fromRgba(fixture.width, fixture.height, source);
      startExport(pngSession, 0, fixture, variant.threshold, 'png');
      while (pngSession.stage() !== JobStage.Ready) pngSession.advance(0, MAX_SAFE);
      const png = concat([...pngSession.drainExport(0), ...pngSession.finishExport(0)]);
      const { width, height } = pngHeaderSize(png);
      assert.deepEqual([...png.subarray(0, 8)], PNG_SIGNATURE, `${label}: PNG signature`);
      pngSession.dispose();
      pngSession.free();

      const outputs = [1, MAX_SAFE].map((budget) => {
        const session = ImageSession.fromRgba(fixture.width, fixture.height, source);
        const text = textOf(session, fixture, variant, budget);
        session.dispose();
        session.free();
        return text;
      });
      assert.equal(Buffer.compare(outputs[0], outputs[1]), 0, `${label}: one-byte and large budgets differ`);

      const text = outputs[0];
      assert.notDeepEqual([...text.subarray(0, 3)], UTF8_BOM, `${label}: no BOM`);
      assert.equal(text.includes(13), false, `${label}: LF only, never CR`);
      assert.equal(text.at(-1), 10, `${label}: the last line ends with LF`);

      const lines = decoder.decode(text).split('\n');
      assert.equal(lines.pop(), '', `${label}: nothing follows the final LF`);
      const columns = lines[0].length;
      assert.ok(columns > 0 && lines.length > 0, `${label}: the grid is not empty`);
      for (const line of lines) {
        assert.equal(line.length, columns, `${label}: every line has the same width`);
      }
      if (fixture.sampling === 'classic') {
        assert.equal(lines.length, height / fixture.cellSize, `${label}: one line per PNG grid row`);
        assert.equal(columns, width / fixture.cellSize, `${label}: each line matches the PNG grid width`);
      }
    }
  }
});

const DEFAULT_FOREGROUND = '#00ff00';
const DEFAULT_BACKGROUND = '#000000';

function textExport(entry, style, budget) {
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  const args = [entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, entry.mode, entry.ramp ?? undefined];
  assert.equal(session.startExport(0, ...args, ...style, 'text'), JobStage.Sampling);
  const text = runExport(session, 0, budget);
  session.dispose();
  session.free();
  return text;
}

test('the reference lists every text case the Chromium and desktop checks need', () => {
  assert.deepEqual(
    reference.text.map((entry) => entry.name),
    ['classic', 'ascii_default', 'ascii_custom', 'block', 'invert_coloured', 'contrast_raised'],
  );
});

test('every text reference has non-empty expected bytes', () => {
  for (const entry of reference.text) {
    assert.ok(bytes(entry.text).length > 0, `${entry.name}: expected text bytes must be non-empty`);
  }
});

for (const entry of reference.text) {
  test(`${entry.name} text export matches the Rust text task bytes for a one-byte and a large budget`, () => {
    const expected = bytes(entry.text);
    const styled = [entry.foreground, entry.background, entry.colorMode, entry.contrast, entry.invert];
    const uncoloured = [DEFAULT_FOREGROUND, DEFAULT_BACKGROUND, 'monochrome', entry.contrast, entry.invert];
    for (const budget of [1, MAX_SAFE]) {
      for (const [label, style] of [['styled', styled], ['uncoloured', uncoloured]]) {
        const text = textExport(entry, style, budget);
        assert.equal(Buffer.compare(text, expected), 0, `${entry.name} ${label}@${budget}: text bytes differ from the Rust task`);
      }
    }
  });
}
