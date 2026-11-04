import assert from 'node:assert/strict';
import { test } from 'node:test';

import { assertCode, gradient, ImageSession, JobStage, newSession, startClassic } from './support.mjs';

const notIntegers = [NaN, Infinity, -Infinity, -251, -1, 1.5, '20', null, undefined, true, false, {}, [], 2 ** 53, 2 ** 64];

test('source construction validates numbers, type, length and ceilings before copying', () => {
  const pixels = new Uint8Array(16);
  for (const bad of notIntegers) {
    assertCode(() => ImageSession.fromRgba(bad, 2, pixels), 'invalid_argument', `width ${String(bad)}`);
    assertCode(() => ImageSession.fromRgba(2, bad, pixels), 'invalid_argument', `height ${String(bad)}`);
  }
  assertCode(() => ImageSession.fromRgba(0, 2, pixels), 'invalid_dimensions');
  assertCode(() => ImageSession.fromRgba(-0, 2, pixels), 'invalid_dimensions');
  assertCode(() => ImageSession.fromRgba(2 ** 32, 1, pixels), 'resource_limit_exceeded');
  assertCode(() => ImageSession.fromRgba(2, 2, new Uint8Array(15)), 'invalid_dimensions');
  assertCode(() => ImageSession.fromRgba(2, 2, new Uint8Array(17)), 'invalid_dimensions');
  for (const bad of [[...pixels], new Uint8ClampedArray(16), new ArrayBuffer(16), null, undefined]) {
    assertCode(() => ImageSession.fromRgba(2, 2, bad), 'invalid_argument', 'pixels type');
  }
  assertCode(() => ImageSession.fromRgba(10000, 5001, new Uint8Array(4)), 'resource_limit_exceeded', '50 MP before copy');
  assertCode(() => ImageSession.fromRgba(7072, 7072, new Uint8Array(4)), 'resource_limit_exceeded', '50 MP before copy');
  assertCode(() => ImageSession.fromRgbaWithMaxPixels(2, 2, 3, pixels), 'resource_limit_exceeded');
  assertCode(() => ImageSession.fromRgbaWithMaxPixels(2, 2, 0, pixels), 'invalid_settings');
  assertCode(() => ImageSession.fromRgbaWithMaxPixels(2, 2, 50_000_001, pixels), 'invalid_settings');
  assertCode(() => ImageSession.fromRgbaWithMaxPixels(2, 2, '4', pixels), 'invalid_argument');
  ImageSession.fromRgbaWithMaxPixels(2, 2, 4, pixels).free();
  ImageSession.fromRgba(2, 2, Buffer.alloc(16)).free();
});

test('settings, generation and budget never coerce or wrap', () => {
  const session = newSession();
  const valid = [0, 20, 20, 128, 'classic'];
  for (let position = 0; position < 4; position += 1) {
    for (const bad of notIntegers) {
      const args = [...valid];
      args[position] = bad;
      assertCode(() => session.startPreview(...args), 'invalid_argument', `arg ${position} = ${String(bad)}`);
    }
  }
  const ranges = [[1, 4], [1, 31], [1, 300], [1, 2 ** 40], [2, 7], [2, 31], [2, 266], [3, 256], [3, 384]];
  for (const [position, bad] of ranges) {
    const args = [...valid];
    args[position] = bad;
    assertCode(() => session.startPreview(...args), 'invalid_settings', `arg ${position} = ${bad}`);
  }
  for (const bad of ['Classic', '', 'median']) {
    assertCode(() => session.startPreview(0, 20, 20, 128, bad), 'invalid_settings', String(bad));
  }
  for (const bad of [null, 5, undefined, {}]) {
    assertCode(() => session.startPreview(0, 20, 20, 128, bad), 'invalid_argument', String(bad));
  }
  assert.equal(session.stage(), JobStage.Idle);
  assert.equal(startClassic(session, 0), JobStage.Sampling, 'zero is a valid first generation');
  for (const bad of notIntegers) {
    assertCode(() => session.advance(bad, 1), 'invalid_argument');
    assertCode(() => session.cancel(bad), 'invalid_argument');
    assertCode(() => session.finish(bad), 'invalid_argument');
  }
  for (const bad of [0, -0, ...notIntegers]) {
    assertCode(() => session.advance(0, bad), 'invalid_argument', `budget ${String(bad)}`);
  }
  assert.equal(session.progressCompleted(), 0);
  session.free();
});

test('source pixels are copied at construction', async () => {
  const source = gradient(30, 30);
  const session = ImageSession.fromRgba(30, 30, source);
  source.fill(255);
  startClassic(session, 0);
  while (session.stage() !== JobStage.Ready) session.advance(0, 4096);
  const frame = session.finish(0);
  const reference = ImageSession.fromRgba(30, 30, gradient(30, 30));
  startClassic(reference, 0);
  while (reference.stage() !== JobStage.Ready) reference.advance(0, 4096);
  const expected = reference.finish(0);
  const actualPixels = frame.takePixels();
  const expectedPixels = expected.takePixels();
  frame.free();
  expected.free();
  reference.free();
  session.free();
  assert.equal(Buffer.compare(actualPixels, expectedPixels), 0);
});
