import assert from 'node:assert/strict';
import { test } from 'node:test';

import { assertCode, bytes, drive, ImageSession, JobStage, loadReference, MAX_SAFE, startClassic } from './support.mjs';

const reference = await loadReference();
const DEFAULT_RAMP = ' .:-=+*#%@';

function concat(chunks) {
  const out = new Uint8Array(chunks.reduce((sum, chunk) => sum + chunk.length, 0));
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

function sameBytes(actual, expected, label) {
  assert.equal(actual.length, expected.length, `${label}: length`);
  assert.equal(Buffer.compare(Buffer.from(actual), Buffer.from(expected)), 0, `${label}: bytes differ`);
}

function errorOf(fn) {
  try {
    fn();
  } catch (error) {
    return error;
  }
  assert.fail('expected the call to throw');
}

for (const entry of reference.modes) {
  test(`${entry.name} previews and exports byte-identical to the native reference`, () => {
    const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
    const args = [entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, entry.mode, entry.ramp ?? undefined];
    assert.equal(session.startPreview(0, ...args), JobStage.Sampling);
    drive(session, 0, 3);
    const frame = session.finish(0);
    assert.equal(frame.width, entry.previewWidth);
    assert.equal(frame.height, entry.previewHeight);
    sameBytes(frame.takePixels(), bytes(entry.preview), `${entry.name} preview`);
    frame.free();

    const exportStage = session.startExport(1, ...args);
    assert.ok([JobStage.Sampling, JobStage.Encoding].includes(exportStage), 'export starts sampling or encoding');
    const drained = [];
    while (session.stage() !== JobStage.Ready) {
      session.advance(1, 2);
      drained.push(...session.drainExport(1));
    }
    drained.push(...session.finishExport(1));
    sameBytes(concat(drained), bytes(entry.export), `${entry.name} export`);
    session.dispose();
    session.free();
  });
}

test('omitted mode and ramp are the classic binary path, byte for byte', () => {
  const entry = reference.cases[0];
  const variant = entry.variants[0];
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  session.startPreview(0, entry.cellSize, entry.fontSize, variant.threshold, entry.sampling);
  drive(session, 0, 7);
  const frame = session.finish(0);
  sameBytes(frame.takePixels(), bytes(variant.preview), 'classic preview');
  frame.free();
  session.dispose();
  session.free();
});

test('explicit binary and an undefined ramp match the omitted arguments', () => {
  const entry = reference.modes[0];
  const sources = [undefined, 'binary'].map((mode) => {
    const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
    session.startPreview(0, entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, mode, undefined);
    drive(session, 0, MAX_SAFE);
    const frame = session.finish(0);
    const pixels = frame.takePixels();
    frame.free();
    session.dispose();
    session.free();
    return pixels;
  });
  sameBytes(sources[1], sources[0], 'explicit binary');
  sameBytes(sources[0], bytes(entry.preview), 'binary reference');
});

test('omitted ascii ramp selects the shared default ramp', () => {
  const entry = reference.modes[1];
  assert.equal(entry.ramp, DEFAULT_RAMP);
  const render = (ramp) => {
    const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
    session.startPreview(0, entry.cellSize, entry.fontSize, entry.threshold, 'average', 'ascii', ramp);
    drive(session, 0, MAX_SAFE);
    const frame = session.finish(0);
    const pixels = frame.takePixels();
    frame.free();
    session.dispose();
    session.free();
    return pixels;
  };
  sameBytes(render(undefined), render(DEFAULT_RAMP), 'default ramp');
});

test('invalid modes and ramps return typed errors and keep the current job', () => {
  const entry = reference.modes[2];
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  startClassic(session, 0);
  session.advance(0, 3);
  const before = session.progressCompleted();
  const oversized = 'a'.repeat(65);
  const cases = [
    ['ascii', '', 'invalid_settings', 'ramp must contain at least one character', 'average'],
    ['ascii', 'é', 'invalid_settings', 'ramp must contain only printable ASCII characters', 'average'],
    ['ascii', 'a\tb', 'invalid_settings', 'ramp must contain only printable ASCII characters', 'average'],
    ['ascii', oversized, 'invalid_settings', 'ramp must be at most 64 characters', 'average'],
    ['ascii', 5, 'invalid_argument', 'ramp must be a string', 'average'],
    ['ascii', ' .#', 'invalid_settings', 'ascii and block modes require average sampling', 'classic'],
    ['block', ' .#', 'invalid_settings', 'ascii and block modes require average sampling', 'classic'],
    ['dots', undefined, 'invalid_settings', 'mode must be "binary", "ascii" or "block"', 'average'],
    [7, undefined, 'invalid_argument', 'mode must be a string', 'average'],
  ];
  for (const [mode, ramp, code, message, sampling] of cases) {
    const error = errorOf(() => session.startExport(1, 5, 10, 128, sampling, mode, ramp));
    assert.equal(error.code, code, `${String(mode)} ${JSON.stringify(ramp)}`);
    assert.equal(error.message, message);
    assert.equal(session.stage(), JobStage.Sampling, 'the failed start keeps the job');
  }
  assert.equal(session.progressCompleted(), before, 'the failed starts spend no work');
  session.advance(0, 1);
  session.dispose();
  session.free();
});

test('the ramp is validated in every mode, so block and binary refuse an invalid ramp', () => {
  const entry = reference.modes[3];
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  for (const mode of ['block', 'binary']) {
    const sampling = mode === 'binary' ? 'classic' : 'average';
    const error = errorOf(() => session.startPreview(0, entry.cellSize, entry.fontSize, entry.threshold, sampling, mode, ''));
    assert.equal(error.code, 'invalid_settings', mode);
    assert.equal(error.message, 'ramp must contain at least one character', mode);
  }
  assert.equal(session.stage(), JobStage.Idle, 'refused starts leave no job');
  assert.equal(session.startPreview(0, entry.cellSize, entry.fontSize, entry.threshold, 'average', 'block', undefined), JobStage.Sampling);
  drive(session, 0, MAX_SAFE);
  const frame = session.finish(0);
  sameBytes(frame.takePixels(), bytes(entry.preview), 'block with the default ramp');
  frame.free();
  session.dispose();
  session.free();
});

test('the generation rule applies to mode starts: a stale mode start is refused', () => {
  const entry = reference.modes[1];
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  session.startPreview(3, entry.cellSize, entry.fontSize, entry.threshold, 'average', 'ascii', undefined);
  assertCode(
    () => session.startPreview(3, entry.cellSize, entry.fontSize, entry.threshold, 'average', 'block', undefined),
    'stale_generation',
    'same generation',
  );
  session.dispose();
  session.free();
});
