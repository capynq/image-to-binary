import assert from 'node:assert/strict';
import { test } from 'node:test';

import { assertCode, bytes, drive, gradient, ImageSession, instance, JobStage, loadReference, MAX_SAFE, startClassic } from './support.mjs';

const reference = await loadReference();
const PNG_SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10];
const IEND_CHUNK = [0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130];

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

function sameBytes(actual, expected, label) {
  assert.equal(actual.length, expected.length, `${label}: length`);
  assert.equal(Buffer.compare(actual, expected), 0, `${label}: bytes differ`);
}

function assertPngShape(png, label) {
  assert.deepEqual([...png.subarray(0, 8)], PNG_SIGNATURE, `${label}: signature`);
  assert.deepEqual([...png.subarray(png.length - 12)], IEND_CHUNK, `${label}: IEND trailer`);
}

/** Advances an export to Ready, draining after every advance, then finishes it. */
function runExport(session, generation, budget) {
  const stages = [session.stage()];
  const drained = [];
  while (session.stage() !== JobStage.Ready) {
    session.advance(generation, budget);
    if (session.stage() !== stages.at(-1)) stages.push(session.stage());
    drained.push(...session.drainExport(generation));
  }
  const final = session.finishExport(generation);
  return { stages, drained, final };
}

function pngSize(png) {
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
  return { width: view.getUint32(16), height: view.getUint32(20) };
}

/** Advances past sampling so the session is in `Encoding`. */
function toEncoding(session, generation) {
  while (session.stage() === JobStage.Sampling) session.advance(generation, MAX_SAFE);
  assert.equal(session.stage(), JobStage.Encoding);
}

/** One grid row is `frame width * tile height` pixels; the frame size comes from the PNG header. */
function rowPixels(session, png) {
  const { width, height } = pngSize(png);
  return width * Math.floor(height / session.progressTotal());
}

function startExport(session, generation, variant, fixture) {
  return session.startExport(generation, fixture.cellSize, fixture.fontSize, variant.threshold, fixture.sampling);
}

test('exports are byte-identical to native PNGs for every fixture, variant and row budget', () => {
  for (const fixture of reference.cases) {
    for (const variant of fixture.variants) {
      const label = `${fixture.name}@${variant.threshold}`;
      const expected = bytes(variant.export);
      const probe = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
      startExport(probe, 0, variant, fixture);
      toEncoding(probe, 0);
      const row = rowPixels(probe, expected);
      probe.dispose();
      probe.free();
      const budgets = [1, Math.max(row - 1, 1), row, 3 * row + 1, 17 * row, MAX_SAFE];
      for (const budget of budgets) {
        const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
        const stage = startExport(session, 0, variant, fixture);
        assert.equal(stage, JobStage.Sampling, label);
        const { stages, drained, final } = runExport(session, 0, budget);
        assert.deepEqual(stages, [JobStage.Sampling, JobStage.Encoding, JobStage.Ready], `${label}@${budget}`);
        const png = concat([...drained, ...final]);
        sameBytes(png, expected, `${label}@${budget}`);
        assertPngShape(png, `${label}@${budget}`);
        session.dispose();
        session.free();
      }
    }
  }
});

test('large exports span several browser chunks and match native output for both samplings', () => {
  for (const entry of reference.large) {
    const session = ImageSession.fromRgba(entry.width, entry.height, largePixels(entry.width));
    assert.equal(session.startExport(0, entry.cellSize, entry.fontSize, entry.threshold, entry.sampling), JobStage.Sampling);
    const { drained, final } = runExport(session, 0, 1_000_000);
    assert.ok(drained.length > 0, `${entry.name}: expected drained chunks before finish`);
    for (const chunk of drained) {
      assert.ok(chunk instanceof Uint8Array, `${entry.name}: chunks are Uint8Array`);
      assert.ok(chunk.length > 0 && chunk.length <= 1024 * 1024, `${entry.name}: chunk size`);
    }
    const png = concat([...drained, ...final]);
    sameBytes(png, bytes(entry.export), entry.name);
    assertPngShape(png, entry.name);
    session.dispose();
    session.free();
  }
});

test('drained chunks stay valid and unchanged after later calls, memory growth and disposal', () => {
  const entry = reference.large[0];
  const session = ImageSession.fromRgba(entry.width, entry.height, largePixels(entry.width));
  session.startExport(0, entry.cellSize, entry.fontSize, entry.threshold, entry.sampling);
  const drained = [];
  while (session.stage() !== JobStage.Ready) {
    session.advance(0, 17);
    drained.push(...session.drainExport(0));
  }
  const snapshots = drained.map((chunk) => chunk.slice());
  assert.ok(drained.length > 0);
  const before = instance.memory.buffer.byteLength;
  const other = ImageSession.fromRgba(1600, 1600, gradient(1600, 1600));
  startClassic(other, 0);
  while (other.stage() !== JobStage.Ready) other.advance(0, MAX_SAFE);
  other.finish(0).free();
  assert.ok(instance.memory.buffer.byteLength > before, 'the second session must grow WASM memory');
  for (const chunk of drained) assert.notEqual(chunk.buffer, instance.memory.buffer);
  const final = session.finishExport(0);
  session.dispose();
  other.dispose();
  other.free();
  session.free();
  drained.forEach((chunk, index) => sameBytes(chunk, snapshots[index], `chunk ${index} after dispose`));
  sameBytes(concat([...drained, ...final]), bytes(entry.export), 'large export after reuse');
});

/** Same deterministic pixels as `large_rgba` in reference_fixtures.rs. */
function largePixels(side) {
  const pixels = new Uint8Array(side * side * 4);
  for (let y = 0; y < side; y += 1) {
    for (let x = 0; x < side; x += 1) {
      const offset = (y * side + x) * 4;
      const red = ((x * 131 + y * 137) ^ (x * y * 7)) % 256;
      const green = ((x * x + y * 3) ^ (y * y * 5)) % 256;
      const blue = ((x ^ y) * 29 + ((x * 11 + y * 13) % 61)) % 256;
      pixels.set([red, green, blue, 255], offset);
    }
  }
  return pixels;
}

test('each chunk is delivered exactly once and a second drain returns nothing new', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  const drained = [];
  while (session.stage() !== JobStage.Ready) {
    session.advance(0, 2);
    drained.push(...session.drainExport(0));
    assert.equal(session.drainExport(0).length, 0);
  }
  assert.equal(session.drainExport(0).length, 0);
  const final = session.finishExport(0);
  assert.ok(final.length > 0);
  assertCode(() => session.finishExport(0), 'no_job', 'second finish');
  assert.equal(session.stage(), JobStage.Idle);
  sameBytes(concat([...drained, ...final]), bytes(variant.export), 'exactly once');
  session.dispose();
  session.free();
});

test('zero and negative row budgets are invalid arguments that leave the job unchanged', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  for (const budget of [0, -1, 1.5, NaN]) {
    assertCode(() => session.advance(0, budget), 'invalid_argument', `budget ${budget}`);
  }
  assert.equal(session.stage(), JobStage.Sampling);
  session.advance(0, MAX_SAFE);
  assert.equal(session.stage(), JobStage.Encoding);
  assertCode(() => session.advance(0, 0), 'invalid_argument', 'encoding zero budget');
  assert.equal(session.stage(), JobStage.Encoding);
  assert.equal(session.progressCompleted(), 0);
  session.dispose();
  session.free();
});

test('stale generations, cancellation and disposal never publish chunks or results', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  session.advance(0, MAX_SAFE);
  session.advance(0, 2);
  assertCode(() => startExport(session, 0, variant, fixture), 'stale_generation', 'same generation');
  assertCode(() => session.drainExport(1), 'stale_generation', 'future drain');
  assertCode(() => session.finishExport(1), 'stale_generation', 'future finish');
  assertCode(() => session.advance(1, 1), 'stale_generation', 'future advance');
  session.cancel(0);
  session.cancel(0);
  assert.equal(session.stage(), JobStage.Failed);
  assertCode(() => session.drainExport(0), 'cancelled', 'drain after cancel');
  assertCode(() => session.finishExport(0), 'cancelled', 'finish after cancel');
  assertCode(() => session.advance(0, 1), 'cancelled', 'advance after cancel');
  session.dispose();
  session.free();
});

test('a preview superseding an export makes every export call stale', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  session.advance(0, 3);
  assert.equal(session.startPreview(1, 6, 10, 128, 'classic'), JobStage.Sampling);
  assertCode(() => session.drainExport(0), 'stale_generation', 'drain after preview');
  assertCode(() => session.finishExport(0), 'stale_generation', 'finish after preview');
  assertCode(() => session.advance(0, 1), 'stale_generation', 'advance after preview');
  while (session.stage() !== JobStage.Ready) session.advance(1, MAX_SAFE);
  const frame = session.finish(1);
  assert.ok(frame.width > 0);
  frame.free();
  session.dispose();
  session.free();
});

test('an export superseding a preview makes the preview calls stale and publishes only the export', () => {
  const fixture = reference.cases[1];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  session.startPreview(0, fixture.cellSize, fixture.fontSize, 128, fixture.sampling);
  session.advance(0, 3);
  assert.equal(startExport(session, 1, variant, fixture), JobStage.Sampling);
  assertCode(() => session.advance(0, 1), 'stale_generation', 'preview advance');
  assertCode(() => session.finish(0), 'stale_generation', 'preview finish');
  const { drained, final } = runExport(session, 1, MAX_SAFE);
  sameBytes(concat([...drained, ...final]), bytes(variant.export), 'export after preview');
  session.dispose();
  session.free();
});

test('disposal mid-export makes every export call disposed', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  session.advance(0, MAX_SAFE);
  session.advance(0, 2);
  session.dispose();
  session.dispose();
  assertCode(() => session.drainExport(0), 'disposed', 'drain');
  assertCode(() => session.finishExport(0), 'disposed', 'finish');
  assertCode(() => session.advance(0, 1), 'disposed', 'advance');
  assertCode(() => startExport(session, 1, variant, fixture), 'disposed', 'start');
  assertCode(() => session.stage(), 'disposed', 'stage');
  session.free();
});

test('finishing before the last row is job_incomplete and a finished export reports no_job', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  assertCode(() => session.finishExport(0), 'job_incomplete', 'while sampling');
  session.advance(0, MAX_SAFE);
  assertCode(() => session.finishExport(0), 'job_incomplete', 'before encoding');
  session.advance(0, 1);
  assertCode(() => session.finishExport(0), 'job_incomplete', 'partly encoded');
  assert.equal(session.stage(), JobStage.Encoding);
  session.advance(0, MAX_SAFE);
  assert.ok(session.finishExport(0).length > 0);
  assertCode(() => session.finishExport(0), 'no_job', 'double finish');
  session.dispose();
  session.free();
});

test('export and preview calls refuse the other job kind without disturbing it', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  session.advance(0, MAX_SAFE);
  assertCode(() => session.finish(0), 'no_job', 'preview finish on export');
  assert.equal(session.stage(), JobStage.Encoding);
  session.advance(0, MAX_SAFE);
  assertCode(() => session.finish(0), 'no_job', 'preview finish on ready export');
  assert.equal(session.stage(), JobStage.Ready);
  const drained = session.drainExport(0);
  const final = session.finishExport(0);
  sameBytes(concat([...drained, ...final]), bytes(variant.export), 'export still intact');
  session.startPreview(1, fixture.cellSize, fixture.fontSize, 128, fixture.sampling);
  assertCode(() => session.drainExport(1), 'no_job', 'drain on preview');
  assertCode(() => session.finishExport(1), 'no_job', 'finish export on preview');
  session.dispose();
  session.free();
});

test('export failures on invalid settings keep the current job', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  assertCode(() => session.startExport(1, 4, 10, 128, 'classic'), 'invalid_settings', 'cell');
  assertCode(() => session.startExport(1, 5, 10, 128, 'median'), 'invalid_settings', 'sampling');
  assertCode(() => session.startExport(1, 5, 10, 128, undefined), 'invalid_argument', 'missing sampling');
  assert.equal(session.stage(), JobStage.Sampling);
  const { drained, final } = runExport(session, 0, MAX_SAFE);
  sameBytes(concat([...drained, ...final]), bytes(variant.export), 'job survived');
  session.dispose();
  session.free();
});

test('exports reuse the grid cached by a preview and cancelled exports restart with a newer generation', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  session.startPreview(0, fixture.cellSize, fixture.fontSize, 128, fixture.sampling);
  drive(session, 0, MAX_SAFE);
  session.finish(0).free();
  assert.equal(startExport(session, 1, variant, fixture), JobStage.Encoding);
  session.advance(1, 1);
  session.cancel(1);
  assertCode(() => startExport(session, 1, variant, fixture), 'stale_generation', 'same generation after cancel');
  assert.equal(startExport(session, 2, variant, fixture), JobStage.Encoding);
  const { drained, final } = runExport(session, 2, 5);
  sameBytes(concat([...drained, ...final]), bytes(variant.export), 'restarted export');
  session.dispose();
  session.free();
});

test('a pixel budget converts to whole grid rows, never fewer than one', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const expected = bytes(variant.export);
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  toEncoding(session, 0);
  const row = rowPixels(session, expected);
  assert.equal(session.advance(0, 1), JobStage.Encoding);
  assert.equal(session.progressCompleted(), 1, 'a tiny budget encodes exactly one row');
  assert.equal(session.advance(0, 3 * row + row - 1), JobStage.Encoding);
  assert.equal(session.progressCompleted(), 4, 'N rows of pixels encode at most N rows');
  assert.equal(session.advance(0, MAX_SAFE), JobStage.Ready);
  assert.equal(session.progressCompleted(), session.progressTotal());
  session.dispose();
  session.free();
});

test('a mid-sized pixel budget needs several advances and cancelling between them publishes nothing', () => {
  const entry = reference.large[0];
  const expected = bytes(entry.export);
  const session = ImageSession.fromRgba(entry.width, entry.height, largePixels(entry.width));
  session.startExport(0, entry.cellSize, entry.fontSize, entry.threshold, entry.sampling);
  toEncoding(session, 0);
  const budget = 20 * rowPixels(session, expected);
  const drained = [];
  assert.equal(session.advance(0, budget), JobStage.Encoding);
  assert.equal(session.progressCompleted(), 20);
  drained.push(...session.drainExport(0));
  session.cancel(0);
  assertCode(() => session.advance(0, budget), 'cancelled', 'advance after mid-export cancel');
  assertCode(() => session.drainExport(0), 'cancelled', 'drain after mid-export cancel');
  assertCode(() => session.finishExport(0), 'cancelled', 'finish after mid-export cancel');
  session.dispose();
  session.free();

  const fresh = ImageSession.fromRgba(entry.width, entry.height, largePixels(entry.width));
  fresh.startExport(1, entry.cellSize, entry.fontSize, entry.threshold, entry.sampling);
  toEncoding(fresh, 1);
  let advances = 0;
  const rest = [];
  while (fresh.stage() !== JobStage.Ready) {
    fresh.advance(1, budget);
    advances += 1;
    rest.push(...fresh.drainExport(1));
  }
  assert.ok(advances > 1, `expected several advances, saw ${advances}`);
  sameBytes(concat([...rest, ...fresh.finishExport(1)]), expected, 'restarted mid-sized export');
  fresh.dispose();
  fresh.free();
});

test('wrong-kind calls report no_job even after the other job was cancelled', () => {
  const fixture = reference.cases[0];
  const variant = fixture.variants[0];
  const preview = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  preview.startPreview(0, fixture.cellSize, fixture.fontSize, 128, fixture.sampling);
  preview.cancel(0);
  assertCode(() => preview.drainExport(0), 'no_job', 'drain on cancelled preview');
  assertCode(() => preview.finishExport(0), 'no_job', 'finish export on cancelled preview');
  assertCode(() => preview.finish(0), 'cancelled', 'own failure still reported');
  assert.equal(preview.stage(), JobStage.Failed);
  preview.dispose();
  preview.free();

  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  startExport(session, 0, variant, fixture);
  session.cancel(0);
  assertCode(() => session.finish(0), 'no_job', 'preview finish on cancelled export');
  assertCode(() => session.finishExport(0), 'cancelled', 'own failure still reported');
  assert.equal(session.stage(), JobStage.Failed, 'the cancelled export stays in the slot');
  assertCode(() => session.drainExport(0), 'cancelled', 'drain after finish reports the failure');
  assertCode(() => startExport(session, 0, variant, fixture), 'stale_generation', 'same generation');
  assert.equal(startExport(session, 1, variant, fixture), JobStage.Sampling);
  const { drained, final } = runExport(session, 1, MAX_SAFE);
  sameBytes(concat([...drained, ...final]), bytes(variant.export), 'replacement after failure');
  session.dispose();
  session.free();
});
