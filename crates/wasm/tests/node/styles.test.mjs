import assert from 'node:assert/strict';
import { test } from 'node:test';

import { assertCode, bytes, drive, ImageSession, JobStage, loadReference, MAX_SAFE } from './support.mjs';

const reference = await loadReference();

const DEFAULT_STYLE = ['#00ff00', '#000000', 'monochrome', 0, false];

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

function styleArgs(entry) {
  return [entry.foreground, entry.background, entry.colorMode, entry.contrast, entry.invert];
}

function previewWith(session, generation, cellSize, fontSize, threshold, sampling, mode, ramp, style) {
  const stage = session.startPreview(generation, cellSize, fontSize, threshold, sampling, mode, ramp, ...style);
  assert.equal(stage, JobStage.Sampling, 'the first start of a grid samples it');
  drive(session, generation, MAX_SAFE);
  const frame = session.finish(generation);
  const pixels = frame.takePixels();
  frame.free();
  return pixels;
}

test('the reference lists every style case the Chromium and desktop checks need', () => {
  assert.deepEqual(
    reference.styles.map((entry) => entry.name),
    ['mono_custom_colours', 'source_colour', 'contrast_raised', 'invert', 'invert_threshold'],
  );
});

for (const entry of reference.styles) {
  test(`${entry.name} previews and exports byte-identical to the native reference`, () => {
    const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
    const args = [entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, 'binary', undefined, ...styleArgs(entry)];
    assert.equal(session.startPreview(0, ...args), JobStage.Sampling);
    drive(session, 0, MAX_SAFE);
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

test('omitted style arguments and the explicit defaults give the same bytes', () => {
  const entry = reference.modes[0];
  const omitted = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  const explicit = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  const base = [entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, 'binary', undefined];
  const omittedPixels = previewWith(omitted, 0, ...base, []);
  const explicitPixels = previewWith(explicit, 0, ...base, DEFAULT_STYLE);
  sameBytes(explicitPixels, omittedPixels, 'explicit defaults');
  sameBytes(omittedPixels, bytes(entry.preview), 'classic reference');
  for (const session of [omitted, explicit]) {
    session.dispose();
    session.free();
  }
});

test('a style-only change reuses the sampled grid: the restart reports Preview, not Sampling', () => {
  const entry = reference.styles[0];
  const base = [entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, 'binary', undefined];
  const restyle = ['#ffffff', '#101010', 'source', 40, true];
  const cached = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  previewWith(cached, 0, ...base, DEFAULT_STYLE);
  assert.equal(cached.startPreview(1, ...base, ...restyle), JobStage.Preview, 'a grid hit skips sampling');
  drive(cached, 1, MAX_SAFE);
  const reused = cached.finish(1);
  const reusedPixels = reused.takePixels();
  reused.free();

  const fresh = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  const freshPixels = previewWith(fresh, 0, ...base, restyle);
  sameBytes(reusedPixels, freshPixels, 'cache hit against a fresh sample');
  for (const session of [cached, fresh]) {
    session.dispose();
    session.free();
  }
});

test('invalid style values return typed errors and keep the current job', () => {
  const entry = reference.styles[0];
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  const base = [entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, 'binary', undefined];
  assert.equal(session.startPreview(0, ...base, ...DEFAULT_STYLE), JobStage.Sampling);
  session.advance(0, 3);
  const before = session.progressCompleted();
  const cases = [
    ['#fff', DEFAULT_STYLE[1], 'monochrome', 0, false, 'invalid_settings', 'foreground must be a #RRGGBB colour'],
    [DEFAULT_STYLE[0], 'rgb(0,0,0)', 'monochrome', 0, false, 'invalid_settings', 'background must be a #RRGGBB colour'],
    [DEFAULT_STYLE[0], DEFAULT_STYLE[1], 'Source', 0, false, 'invalid_settings', 'colorMode must be "monochrome" or "source"'],
    [DEFAULT_STYLE[0], DEFAULT_STYLE[1], 'monochrome', 101, false, 'invalid_settings', 'contrast must be between -100 and 100'],
    [DEFAULT_STYLE[0], DEFAULT_STYLE[1], 'monochrome', 1.5, false, 'invalid_argument', 'contrast must be an integer'],
    [DEFAULT_STYLE[0], DEFAULT_STYLE[1], 'monochrome', 0, 'yes', 'invalid_argument', 'invert must be a boolean'],
    [7, DEFAULT_STYLE[1], 'monochrome', 0, false, 'invalid_argument', 'foreground must be a string'],
  ];
  for (const [foreground, background, colorMode, contrast, invert, code, message] of cases) {
    const error = errorOf(() => session.startExport(1, ...base, foreground, background, colorMode, contrast, invert));
    assert.equal(error.code, code, `${String(foreground)} ${String(colorMode)} ${String(contrast)}`);
    assert.equal(error.message, message);
    assert.equal(session.stage(), JobStage.Sampling, 'the failed start keeps the job');
  }
  assert.equal(session.progressCompleted(), before, 'the failed starts spend no work');
  session.dispose();
  session.free();
});

test('a disposed session reports disposed before any style argument is read', () => {
  const entry = reference.styles[0];
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  session.dispose();
  assertCode(
    () => session.startPreview(0, entry.cellSize, entry.fontSize, entry.threshold, entry.sampling, 'binary', undefined, '#zzzzzz'),
    'disposed',
    'disposed session',
  );
  session.free();
});
