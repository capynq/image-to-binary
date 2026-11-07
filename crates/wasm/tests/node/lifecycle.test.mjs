import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  assertCode,
  drive,
  ImageSession,
  instance,
  JobStage,
  MAX_SAFE,
  newSession,
  packageDir,
  startClassic,
} from './support.mjs';

test('lifecycle errors are binding errors with stable codes', () => {
  const session = newSession();
  assertCode(() => session.advance(0, 1), 'no_job');
  assertCode(() => session.finish(0), 'no_job');
  assertCode(() => session.cancel(0), 'no_job');
  assert.equal(session.stage(), JobStage.Idle);
  startClassic(session, 0);
  assertCode(() => session.finish(0), 'job_incomplete');
  assert.equal(session.stage(), JobStage.Sampling);
  drive(session, 0, 1000);
  const frame = session.finish(0);
  assertCode(() => session.finish(0), 'no_job');
  assert.equal(session.stage(), JobStage.Idle);
  session.cancel(0);
  const pixels = frame.takePixels();
  assert.equal(pixels.length, frame.width * frame.height * 4);
  assertCode(() => frame.takePixels(), 'frame_consumed');
  frame.free();
  session.free();
});

test('cancel before, during and after completion is terminal and a newer job recovers', () => {
  for (const when of ['before', 'sampling', 'preview', 'ready']) {
    const session = newSession();
    startClassic(session, 0);
    if (when === 'sampling') session.advance(0, 3);
    if (when === 'preview') {
      session.advance(0, MAX_SAFE);
      assert.equal(session.advance(0, 5), JobStage.Preview);
    }
    if (when === 'ready') drive(session, 0, MAX_SAFE);
    session.cancel(0);
    session.cancel(0);
    assert.equal(session.stage(), JobStage.Failed, when);
    assertCode(() => session.advance(0, 1), 'cancelled', when);
    assertCode(() => session.finish(0), 'cancelled', when);
    assertCode(() => startClassic(session, 0), 'stale_generation', when);
    startClassic(session, 1);
    drive(session, 1, MAX_SAFE);
    const frame = session.finish(1);
    assert.ok(frame.width > 0 && frame.height > 0);
    frame.free();
    session.free();
  }
});

test('cancelled sampling leaves no partial cache entry', () => {
  const session = newSession();
  startClassic(session, 0);
  session.advance(0, 1);
  session.cancel(0);
  assert.equal(startClassic(session, 1), JobStage.Sampling);
  session.free();
});

test('stale generations are rejected without disturbing or reviving jobs', () => {
  const session = newSession();
  startClassic(session, 5);
  session.advance(5, 3);
  for (const generation of [4, 5]) assertCode(() => startClassic(session, generation), 'stale_generation');
  for (const generation of [4, 6]) {
    assertCode(() => session.advance(generation, 1), 'stale_generation');
    assertCode(() => session.finish(generation), 'stale_generation');
    assertCode(() => session.cancel(generation), 'stale_generation');
  }
  assert.equal(session.stage(), JobStage.Sampling);
  assert.equal(session.progressCompleted(), 3);
  startClassic(session, 7);
  assertCode(() => session.advance(5, 1), 'stale_generation');
  drive(session, 7, MAX_SAFE);
  session.finish(7).free();
  session.free();
});

test('dispose is idempotent, rejects later work, and free invalidates the handle', () => {
  const session = newSession();
  startClassic(session, 0);
  drive(session, 0, MAX_SAFE);
  const frame = session.finish(0);
  session.dispose();
  session.dispose();
  assertCode(() => startClassic(session, 1), 'disposed');
  assertCode(() => session.advance(0, 1), 'disposed');
  assertCode(() => session.finish(0), 'disposed');
  assertCode(() => session.cancel(0), 'disposed');
  assertCode(() => session.stage(), 'disposed');
  assertCode(() => session.progressCompleted(), 'disposed');
  assert.equal(frame.takePixels().length, frame.width * frame.height * 4, 'frames outlive their session');
  frame.free();
  session.free();
  assert.throws(() => session.stage());
});

test('returned pixels are an owned copy that survives later allocations and disposal', () => {
  const session = newSession();
  startClassic(session, 0);
  drive(session, 0, MAX_SAFE);
  const frame = session.finish(0);
  const pixels = frame.takePixels();
  frame.free();
  const snapshot = Uint8Array.from(pixels);
  assert.notEqual(pixels.buffer, instance.memory.buffer);
  assert.ok(pixels.buffer instanceof ArrayBuffer);
  session.dispose();
  session.free();
  const big = ImageSession.fromRgba(2000, 2000, new Uint8Array(2000 * 2000 * 4));
  startClassic(big, 0);
  drive(big, 0, MAX_SAFE);
  big.dispose();
  big.free();
  assert.equal(Buffer.compare(pixels, snapshot), 0);
});

test('returned pixels are independent of explicit WASM memory growth', () => {
  const session = newSession();
  startClassic(session, 0);
  drive(session, 0, MAX_SAFE);
  const frame = session.finish(0);
  const pixels = frame.takePixels();
  frame.free();
  const snapshot = Uint8Array.from(pixels);
  const before = instance.memory.buffer;
  const beforeLength = before.byteLength;
  instance.memory.grow(1);
  const after = instance.memory.buffer;
  assert.notEqual(after, before, 'growth replaces the backing ArrayBuffer');
  assert.equal(after.byteLength, beforeLength + 65536);
  assert.notEqual(pixels.buffer, after);
  assert.equal(Buffer.compare(pixels, snapshot), 0);
  session.dispose();
  session.free();
  assert.equal(Buffer.compare(pixels, snapshot), 0);
});

test('generated declarations expose owned typed-array buffers', async () => {
  const declarations = await readFile(join(packageDir, 'image_to_binary_wasm.d.ts'), 'utf8');
  assert.match(declarations, /takePixels\(\): Uint8Array<ArrayBuffer>/);
  assert.match(declarations, /class ImageSession/);
});

test('generated declarations type numeric and sampling parameters', async () => {
  const declarations = await readFile(join(packageDir, 'image_to_binary_wasm.d.ts'), 'utf8');
  const start = declarations.indexOf('export class ImageSession');
  assert.ok(start >= 0, 'ImageSession declaration');
  const rest = declarations.slice(start);
  const end = rest.indexOf('\nexport ', 1);
  const body = end < 0 ? rest : rest.slice(0, end);
  assert.match(body, /static fromRgba\(\w+: number, \w+: number, \w+: Uint8Array\): ImageSession/);
  assert.match(body, /static fromRgbaWithMaxPixels\(\w+: number, \w+: number, \w+: number, \w+: Uint8Array\): ImageSession/);
  assert.match(
    body,
    /startPreview\(\w+: number, \w+: number, \w+: number, \w+: number, \w+: "classic" \| "average", \w+: "binary" \| "ascii" \| "block" \| undefined, \w+: string \| undefined, \w+: string \| undefined, \w+: string \| undefined, \w+: "monochrome" \| "source" \| undefined, \w+: number \| undefined, \w+: boolean \| undefined\): JobStage/,
  );
  assert.match(body, /advance\(\w+: number, \w+: number\): JobStage/);
  assert.match(body, /cancel\(\w+: number\): void/);
  assert.match(body, /finish\(\w+: number\): PreviewFrame/);
  assert.doesNotMatch(
    body,
    /:\s*any\b|<\s*any\s*>|\bany\s*\[\]|\|\s*any\b|\bany\s*\|/,
    'no untyped parameters or results',
  );
});
