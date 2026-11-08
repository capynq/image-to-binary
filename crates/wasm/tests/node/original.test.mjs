import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  assertCode,
  bytes,
  drive,
  ImageSession,
  instance,
  JobStage,
  loadReference,
  MAX_SAFE,
  newSession,
  packageDir,
  startClassic,
} from './support.mjs';

const reference = await loadReference();
const MAX_EDGE = 1600;

function patterned(width, height) {
  const pixels = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const offset = (y * width + x) * 4;
      pixels[offset] = (x * 7 + y * 3) % 256;
      pixels[offset + 1] = (x * 2 + y * 13 + 1) % 256;
      pixels[offset + 2] = (x * 5 + y * 11 + 2) % 256;
      pixels[offset + 3] = (x * 11 + y * 17) % 256;
    }
  }
  return pixels;
}

function expectedSize(width, height) {
  const longest = Math.max(width, height);
  if (longest <= MAX_EDGE) return [width, height];
  return [
    Math.max(1, Math.floor((width * MAX_EDGE) / longest)),
    Math.max(1, Math.floor((height * MAX_EDGE) / longest)),
  ];
}

function oracle(source, width, height) {
  const [outWidth, outHeight] = expectedSize(width, height);
  const columns = Uint32Array.from({ length: outWidth }, (_, x) => Math.floor(((2 * x + 1) * width) / (2 * outWidth)));
  const rows = Uint32Array.from({ length: outHeight }, (_, y) => Math.floor(((2 * y + 1) * height) / (2 * outHeight)));
  const pixels = new Uint8Array(outWidth * outHeight * 4);
  for (let y = 0; y < outHeight; y += 1) {
    for (let x = 0; x < outWidth; x += 1) {
      const from = (rows[y] * width + columns[x]) * 4;
      pixels.set(source.subarray(from, from + 4), (y * outWidth + x) * 4);
    }
  }
  return { width: outWidth, height: outHeight, pixels };
}

function runOriginal(session, generation, budget) {
  assert.equal(session.startOriginalPreview(generation), JobStage.Preview);
  drive(session, generation, budget);
  const frame = session.finish(generation);
  const result = { width: frame.width, height: frame.height, pixels: frame.takePixels() };
  frame.free();
  return result;
}

function assertFrame(actual, expected, label) {
  assert.equal(actual.width, expected.width, `${label}: width`);
  assert.equal(actual.height, expected.height, `${label}: height`);
  assert.ok(Math.max(actual.width, actual.height) <= MAX_EDGE, `${label}: longest edge`);
  assert.equal(actual.pixels.length, actual.width * actual.height * 4, `${label}: length`);
  assert.equal(Buffer.compare(actual.pixels, expected.pixels), 0, `${label}: pixels differ`);
}

const SHAPES = [
  [17, 9],
  [1600, 2],
  [1601, 2],
  [3000, 4],
  [4000, 1],
  [5, 2400],
  [1, 4000],
  [1700, 40],
  [30, 2000],
];

for (const [width, height] of SHAPES) {
  test(`original preview of ${width}x${height} matches the nearest-center oracle for every budget`, () => {
    const source = patterned(width, height);
    const expected = oracle(source, width, height);
    const session = ImageSession.fromRgba(width, height, source);
    let generation = 0;
    for (const budget of [1, 17, 4096, MAX_SAFE]) {
      const actual = runOriginal(session, generation, budget);
      assertFrame(actual, expected, `${width}x${height} budget ${budget}`);
      generation += 1;
    }
    if (Math.max(width, height) <= MAX_EDGE) {
      assert.equal(Buffer.compare(expected.pixels, source), 0, 'small sources are returned unchanged');
    }
    session.free();
  });
}

test('the largest original preview is bounded at 1600x1600', () => {
  const width = 1601;
  const height = 1601;
  const source = patterned(width, height);
  const session = ImageSession.fromRgba(width, height, source);
  const actual = runOriginal(session, 0, MAX_SAFE);
  assert.equal(actual.width, MAX_EDGE);
  assert.equal(actual.height, MAX_EDGE);
  assert.equal(actual.pixels.length, MAX_EDGE * MAX_EDGE * 4);
  assertFrame(actual, oracle(source, width, height), 'ceiling');
  session.free();
});

test('progress counts output pixels and a started job spends no budget', () => {
  const session = ImageSession.fromRgba(3000, 4, patterned(3000, 4));
  assert.equal(session.startOriginalPreview(0), JobStage.Preview);
  assert.equal(session.stage(), JobStage.Preview);
  assert.equal(session.progressCompleted(), 0);
  assert.equal(session.progressTotal(), 1600 * 2);
  assert.equal(session.advance(0, 100), JobStage.Preview);
  assert.equal(session.progressCompleted(), 100);
  assertCode(() => session.advance(0, 0), 'invalid_argument', 'zero budget');
  assert.equal(session.progressCompleted(), 100, 'zero budget does not mutate');
  session.free();
});

test('encoded fixtures expose the normalized original for every codec, orientation and first frame', () => {
  const names = reference.encoded.map((entry) => entry.name);
  for (const name of ['bmp', 'gif', 'jpeg', 'png', 'tiff', 'webp', 'jpeg_exif_rotated', 'gif_first_frame', 'tiff_first_page']) {
    assert.ok(names.includes(name), name);
  }
  for (const entry of reference.encoded) {
    assert.equal(typeof entry.source, 'string', `${entry.name}: reference.json lacks the normalized source`);
    const native = bytes(entry.source);
    assert.equal(native.length, entry.width * entry.height * 4, entry.name);
    const session = ImageSession.fromEncoded(bytes(entry.bytes));
    const actual = runOriginal(session, 0, 4096);
    assertFrame(actual, oracle(native, entry.width, entry.height), entry.name);
    if (Math.max(entry.width, entry.height) <= MAX_EDGE) {
      assert.equal(Buffer.compare(actual.pixels, native), 0, `${entry.name}: unscaled original equals the native source`);
    }
    session.free();
  }
});

test('RGBA reference cases return their source through the original preview', () => {
  for (const entry of reference.cases) {
    const source = bytes(entry.source);
    const session = ImageSession.fromRgba(entry.width, entry.height, source);
    assertFrame(runOriginal(session, 0, 17), oracle(source, entry.width, entry.height), entry.name);
    session.free();
  }
});

test('artwork previews are unchanged by earlier original jobs', () => {
  const entry = reference.cases[0];
  const variant = entry.variants[0];
  const session = ImageSession.fromRgba(entry.width, entry.height, bytes(entry.source));
  runOriginal(session, 0, MAX_SAFE);
  assert.equal(
    session.startPreview(1, entry.cellSize, entry.fontSize, variant.threshold, entry.sampling),
    JobStage.Sampling,
  );
  drive(session, 1, MAX_SAFE);
  const frame = session.finish(1);
  assert.equal(frame.width, variant.previewWidth);
  assert.equal(frame.height, variant.previewHeight);
  assert.equal(Buffer.compare(frame.takePixels(), bytes(variant.preview)), 0);
  frame.free();
  session.free();
});

test('taken original pixels are owned copies that survive memory growth, free and disposal', () => {
  const source = patterned(64, 48);
  const session = ImageSession.fromRgba(64, 48, source);
  session.startOriginalPreview(0);
  drive(session, 0, MAX_SAFE);
  const frame = session.finish(0);
  const pixels = frame.takePixels();
  const snapshot = Uint8Array.from(pixels);
  assert.notEqual(pixels.buffer, instance.memory.buffer, 'not a view into WASM memory');
  assertCode(() => frame.takePixels(), 'frame_consumed');
  const before = instance.memory.buffer.byteLength;
  const large = ImageSession.fromRgba(2000, 2000, new Uint8Array(2000 * 2000 * 4));
  assert.ok(instance.memory.buffer.byteLength >= before);
  frame.free();
  session.dispose();
  session.free();
  large.dispose();
  large.free();
  assert.equal(Buffer.compare(pixels, snapshot), 0);
  assert.equal(Buffer.compare(pixels, source), 0);
});

test('generations are monotonic across artwork and original starts', () => {
  const session = newSession(41, 29);
  assert.equal(session.startOriginalPreview(5), JobStage.Preview);
  session.advance(5, 3);
  for (const stale of [4, 5]) {
    assertCode(() => session.startOriginalPreview(stale), 'stale_generation', `original ${stale}`);
    assertCode(() => startClassic(session, stale), 'stale_generation', `artwork ${stale}`);
  }
  assert.equal(session.stage(), JobStage.Preview);
  assert.equal(session.progressCompleted(), 3);
  assert.equal(startClassic(session, 6), JobStage.Sampling);
  assertCode(() => session.advance(5, 1), 'stale_generation');
  assert.equal(session.startOriginalPreview(7), JobStage.Preview);
  assertCode(() => startClassic(session, 7), 'stale_generation');
  assertCode(() => session.advance(6, 1), 'stale_generation');
  drive(session, 7, MAX_SAFE);
  session.finish(7).free();
  assertCode(() => session.startOriginalPreview(7), 'stale_generation', 'consumed generation stays used');
  assert.equal(session.startOriginalPreview(8), JobStage.Preview);
  session.free();
});

test('invalid and stale starts keep the accepted original job intact', () => {
  const session = newSession(41, 29);
  session.startOriginalPreview(5);
  session.advance(5, 3);
  const invalid = [Number.NaN, Number.POSITIVE_INFINITY, -1, 1.5, '6', null, undefined, MAX_SAFE + 1];
  for (const bad of invalid) {
    assertCode(() => session.startOriginalPreview(bad), 'invalid_argument', String(bad));
  }
  assertCode(() => session.startOriginalPreview(), 'invalid_argument', 'missing argument');
  assertCode(() => session.startPreview(6, 4, 10, 128, 'classic'), 'invalid_settings');
  assert.equal(session.stage(), JobStage.Preview);
  assert.equal(session.progressCompleted(), 3);
  assert.equal(session.advance(5, 2), JobStage.Preview);
  assert.equal(session.progressCompleted(), 5);
  assert.equal(session.startOriginalPreview(MAX_SAFE), JobStage.Preview);
  assertCode(() => session.startOriginalPreview(MAX_SAFE), 'stale_generation');
  session.free();
});

test('cancel, incomplete finish and dispose follow the shared lifecycle', () => {
  const session = newSession(41, 29);
  assertCode(() => session.cancel(0), 'no_job');
  session.startOriginalPreview(0);
  assertCode(() => session.finish(0), 'job_incomplete');
  session.advance(0, 2);
  session.cancel(0);
  session.cancel(0);
  assert.equal(session.stage(), JobStage.Failed);
  assertCode(() => session.advance(0, 1), 'cancelled');
  assertCode(() => session.finish(0), 'cancelled');
  assertCode(() => session.startOriginalPreview(0), 'stale_generation');
  const recovered = runOriginal(session, 1, 17);
  assert.equal(recovered.width, 41);
  assert.equal(recovered.height, 29);
  assertCode(() => session.finish(1), 'no_job');
  session.startOriginalPreview(2);
  session.dispose();
  session.dispose();
  assertCode(() => session.startOriginalPreview(3), 'disposed');
  assertCode(() => session.advance(2, 1), 'disposed');
  assertCode(() => session.finish(2), 'disposed');
  assertCode(() => session.cancel(2), 'disposed');
  assertCode(() => session.stage(), 'disposed');
  session.free();
});

test('generated declarations type startOriginalPreview as a number-taking stage method', async () => {
  const declarations = await readFile(join(packageDir, 'image_to_binary_wasm.d.ts'), 'utf8');
  const start = declarations.indexOf('export class ImageSession');
  assert.ok(start >= 0, 'ImageSession declaration');
  assert.ok(
    declarations.slice(start).includes('startOriginalPreview(generation: number, frame?: number): JobStage'),
    'startOriginalPreview declaration',
  );
});
