import assert from 'node:assert/strict';
import { test } from 'node:test';

import { bytes, drive, ImageSession, instance, JobStage, loadReference, MAX_SAFE } from './support.mjs';

const reference = await loadReference();

const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const U64_MASK = 0xffffffffffffffffn;

function fnv64(pixels) {
  let hash = FNV_OFFSET;
  for (const byte of pixels) hash = ((hash ^ BigInt(byte)) * FNV_PRIME) & U64_MASK;
  return hash;
}

function assertGreenOnBlack(pixels, label) {
  let green = 0;
  let black = 0;
  for (let i = 0; i < pixels.length; i += 4) {
    if (pixels[i] !== 0 || pixels[i + 2] !== 0 || pixels[i + 3] !== 255) {
      assert.fail(`${label}: pixel ${i / 4} is not opaque green-on-black`);
    }
    if (pixels[i + 1] === 0) black += 1;
    else green += 1;
  }
  return { green, black };
}

for (const budget of [1, 17, 4096, MAX_SAFE]) {
  test(`WASM previews match native references with chunk budget ${budget}`, () => {
    for (const fixture of reference.cases) {
      const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
      fixture.variants.forEach((variant, index) => {
        const label = `${fixture.name}@${variant.threshold}`;
        const stage = session.startPreview(index, fixture.cellSize, fixture.fontSize, variant.threshold, fixture.sampling);
        assert.equal(stage, index === 0 ? JobStage.Sampling : JobStage.Preview, `${label}: sample grid reuse`);
        const stages = drive(session, index, budget);
        const expectedStages = index === 0 ? [JobStage.Sampling, JobStage.Preview, JobStage.Ready] : [JobStage.Preview, JobStage.Ready];
        assert.deepEqual(stages, expectedStages, label);
        const frame = session.finish(index);
        assert.equal(frame.width, variant.previewWidth, label);
        assert.equal(frame.height, variant.previewHeight, label);
        const pixels = frame.takePixels();
        assert.equal(Buffer.compare(pixels, bytes(variant.preview)), 0, `${label}: pixels differ from native`);
        assert.notEqual(pixels.buffer, instance.memory.buffer);
        const counts = assertGreenOnBlack(pixels, label);
        if (!fixture.name.endsWith('_bounded')) {
          assert.ok(counts.green > 0 && counts.black > 0, `${label}: expected glyphs and background`);
        } else {
          assert.equal(Math.max(frame.width, frame.height), 1600, `${label}: bounded preview`);
        }
        frame.free();
      });
      session.dispose();
      session.free();
    }
  });
}

test('changing geometry resamples and returning to a cached geometry reuses its grid', () => {
  const fixture = reference.cases[0];
  const session = ImageSession.fromRgba(fixture.width, fixture.height, bytes(fixture.source));
  const start = (generation, cell) => session.startPreview(generation, cell, fixture.fontSize, 128, 'classic');
  assert.equal(start(0, 7), JobStage.Sampling);
  drive(session, 0, MAX_SAFE);
  assert.equal(start(1, 8), JobStage.Sampling);
  drive(session, 1, MAX_SAFE);
  assert.equal(start(2, 7), JobStage.Preview);
  session.free();
});

test('WASM matches the immutable native 15x15 checker golden hash', () => {
  const source = new Uint8Array(15 * 15 * 4);
  for (let y = 0; y < 15; y += 1) {
    for (let x = 0; x < 15; x += 1) {
      const value = (Math.floor(x / 5) + Math.floor(y / 5)) % 2 === 0 ? 0 : 255;
      const offset = (y * 15 + x) * 4;
      source.set([value, value, value, 255], offset);
    }
  }
  const session = ImageSession.fromRgba(15, 15, source);
  assert.equal(session.startPreview(0, 5, 30, 128, 'classic'), JobStage.Sampling);
  drive(session, 0, MAX_SAFE);
  const frame = session.finish(0);
  assert.equal(frame.width, 15);
  assert.equal(frame.height, 15);
  const pixels = frame.takePixels();
  frame.free();
  session.dispose();
  session.free();
  assert.equal(pixels.length, 15 * 15 * 4);
  assert.equal(fnv64(pixels), 15743520938017280556n);
});
