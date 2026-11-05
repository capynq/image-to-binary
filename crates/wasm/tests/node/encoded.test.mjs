import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  assertCode,
  bytes,
  drive,
  gradient,
  ImageSession,
  instance,
  loadReference,
  MAX_SAFE,
  packageDir,
} from './support.mjs';

const reference = await loadReference();
const CODECS = ['bmp', 'gif', 'jpeg', 'png', 'tiff', 'webp'];
const DECODE_CODES = ['unsupported_format', 'decode_failed', 'resource_limit_exceeded'];
const MAX_ENCODED = 256 * 1024 * 1024;

function fixture(name) {
  const found = reference.encoded.find((entry) => entry.name === name);
  assert.ok(found, `missing generated fixture ${name}`);
  return found;
}

function previewOf(session) {
  session.startPreview(0, 5, 10, 128, 'classic');
  drive(session, 0, MAX_SAFE);
  const frame = session.finish(0);
  const result = { width: frame.width, height: frame.height, pixels: frame.takePixels() };
  frame.free();
  return result;
}

function assertParity(session, expected, label) {
  const actual = previewOf(session);
  assert.equal(actual.width, expected.previewWidth, label);
  assert.equal(actual.height, expected.previewHeight, label);
  assert.equal(Buffer.compare(actual.pixels, bytes(expected.preview)), 0, `${label}: preview differs from the normalized native source`);
}

function codeOf(action) {
  try {
    action();
  } catch (error) {
    assert.ok(!(error instanceof WebAssembly.RuntimeError), 'must not be a trap');
    assert.equal(error.name, 'ImageToBinaryError');
    return error.code;
  }
  return assert.fail('expected a coded error');
}

test('reference JSON carries every generated codec, orientation and page fixture', () => {
  const names = reference.encoded.map((entry) => entry.name);
  for (const name of [...CODECS, 'jpeg_exif_rotated', 'gif_first_frame', 'tiff_first_page']) {
    assert.ok(names.includes(name), name);
  }
  assert.deepEqual(reference.rejected.map((entry) => entry.name).sort(), ['pixel_limit_png', 'truncated_png']);
});

for (const entry of reference.encoded) {
  test(`fromEncoded decodes ${entry.name} once into the normalized source and preview`, () => {
    const session = ImageSession.fromEncoded(bytes(entry.bytes));
    assert.equal(session.sourceWidth, entry.width);
    assert.equal(session.sourceHeight, entry.height);
    assert.equal(session.sourceChannels, 4);
    assert.equal(entry.channels, 4);
    assertParity(session, entry, entry.name);
    assert.equal(session.sourceWidth, entry.width, 'metadata is immutable across jobs');
    assert.equal(session.sourceHeight, entry.height);
    session.dispose();
    for (const getter of ['sourceWidth', 'sourceHeight', 'sourceChannels']) {
      assertCode(() => session[getter], 'disposed', getter);
    }
    session.dispose();
    assertCode(() => session.sourceWidth, 'disposed', 'stable after repeated dispose');
    session.free();
  });
}

test('EXIF orientation swaps the normalized dimensions', () => {
  const upright = fixture('jpeg');
  const rotated = fixture('jpeg_exif_rotated');
  assert.notEqual(upright.width, upright.height);
  assert.equal(rotated.width, upright.height);
  assert.equal(rotated.height, upright.width);
  const session = ImageSession.fromEncoded(bytes(rotated.bytes));
  assert.equal(session.sourceWidth, upright.height);
  assert.equal(session.sourceHeight, upright.width);
  session.free();
});

for (const name of ['gif_first_frame', 'tiff_first_page']) {
  test(`${name} decodes only the first frame or page`, () => {
    const entry = fixture(name);
    const encoded = ImageSession.fromEncoded(bytes(entry.bytes));
    const white = ImageSession.fromRgba(10, 10, new Uint8Array(10 * 10 * 4).fill(255));
    assert.equal(encoded.sourceWidth, 10);
    assert.equal(encoded.sourceHeight, 10);
    assert.deepEqual(previewOf(encoded), previewOf(white));
    encoded.free();
    white.free();
  });
}

test('runtime wrong types are rejected before any copy', () => {
  const before = instance.memory.buffer.byteLength;
  const wrong = [
    undefined,
    null,
    [],
    [1, 2, 3],
    'abc',
    5,
    {},
    { length: 4 },
    new ArrayBuffer(8),
    new Uint8ClampedArray(8),
    new Uint16Array(4),
    new DataView(new ArrayBuffer(8)),
  ];
  for (const bad of wrong) {
    assertCode(() => ImageSession.fromEncoded(bad), 'invalid_argument', String(bad));
  }
  assertCode(() => ImageSession.fromEncoded(), 'invalid_argument', 'missing argument');
  assert.equal(instance.memory.buffer.byteLength, before, 'no WASM memory growth');
});

test('oversized encoded input fails before any copy', () => {
  const oversized = new Uint8Array(MAX_ENCODED + 1);
  const before = instance.memory.buffer.byteLength;
  assertCode(() => ImageSession.fromEncoded(oversized), 'encoded_too_large');
  assert.equal(instance.memory.buffer.byteLength, before, 'no WASM memory growth');
});

test('malformed and over-limit fixtures fail with stable codes and no session', () => {
  for (const entry of reference.rejected) {
    const before = instance.memory.buffer.byteLength;
    assertCode(() => ImageSession.fromEncoded(bytes(entry.bytes)), entry.code, entry.name);
    if (entry.name === 'pixel_limit_png') {
      const growth = instance.memory.buffer.byteLength - before;
      assert.ok(growth < 16 * 1024 * 1024, `${entry.name}: rejected from the header without pixel allocation`);
    }
  }
});

test('unsupported, empty and truncated codec inputs report machine-readable codes', () => {
  assertCode(() => ImageSession.fromEncoded(Buffer.from('%PDF-1.7 not an image')), 'unsupported_format');
  assert.ok(DECODE_CODES.includes(codeOf(() => ImageSession.fromEncoded(new Uint8Array(0)))));
  for (const name of CODECS) {
    const entry = fixture(name);
    const head = bytes(entry.bytes).subarray(0, 12);
    assert.ok(DECODE_CODES.includes(codeOf(() => ImageSession.fromEncoded(head))), `${name}: truncated header`);
  }
  ImageSession.fromEncoded(bytes(fixture('png').bytes)).free();
});

test('caller buffers are copied: not retained, mutated or required after construction', () => {
  const entry = fixture('png');
  const input = bytes(entry.bytes);
  const snapshot = Uint8Array.from(input);
  const session = ImageSession.fromEncoded(input);
  assert.equal(Buffer.compare(input, snapshot), 0, 'input is not mutated');
  input.fill(0);
  structuredClone(input.buffer, { transfer: [input.buffer] });
  assert.equal(input.byteLength, 0, 'input buffer is detached');
  assert.equal(session.sourceWidth, entry.width);
  assertParity(session, entry, 'png after the caller buffer was detached');
  session.free();
});

test('typed array views and Buffers copy only their own range', () => {
  const entry = fixture('webp');
  const payload = bytes(entry.bytes);
  const backing = new Uint8Array(payload.length + 64).fill(0xa5);
  backing.set(payload, 32);
  const view = new Uint8Array(backing.buffer, 32, payload.length);
  const fromView = ImageSession.fromEncoded(view);
  assertParity(fromView, entry, 'view');
  fromView.free();
  const fromBuffer = ImageSession.fromEncoded(Buffer.from(payload));
  assertParity(fromBuffer, entry, 'buffer');
  fromBuffer.free();
});

test('sessions from the same bytes are independent and failures do not disturb later decodes', () => {
  const entry = fixture('bmp');
  const first = ImageSession.fromEncoded(bytes(entry.bytes));
  assertCode(() => ImageSession.fromEncoded(null), 'invalid_argument');
  const second = ImageSession.fromEncoded(bytes(entry.bytes));
  first.dispose();
  assertCode(() => first.sourceWidth, 'disposed');
  assert.equal(second.sourceWidth, entry.width);
  assertParity(second, entry, 'second session');
  first.free();
  second.free();
});

test('RGBA sessions report four-channel metadata and keep their semantics', () => {
  const session = ImageSession.fromRgba(30, 20, gradient(30, 20));
  assert.equal(session.sourceWidth, 30);
  assert.equal(session.sourceHeight, 20);
  assert.equal(session.sourceChannels, 4);
  session.dispose();
  assertCode(() => session.sourceChannels, 'disposed');
  session.free();
});

test('generated declarations expose the encoded entry point and metadata getters', async () => {
  const declarations = await readFile(join(packageDir, 'image_to_binary_wasm.d.ts'), 'utf8');
  const start = declarations.indexOf('export class ImageSession');
  assert.ok(start >= 0, 'ImageSession declaration');
  const rest = declarations.slice(start);
  const end = rest.indexOf('\nexport ', 1);
  const body = end < 0 ? rest : rest.slice(0, end);
  assert.match(body, /static fromEncoded\(\w+: Uint8Array\): ImageSession/);
  assert.match(body, /readonly sourceWidth: number/);
  assert.match(body, /readonly sourceHeight: number/);
  assert.match(body, /readonly sourceChannels: number/);
});
