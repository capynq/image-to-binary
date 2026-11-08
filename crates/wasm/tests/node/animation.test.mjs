import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  assertCode,
  bytes,
  drive,
  ImageSession,
  JobStage,
  loadReference,
  MAX_SAFE,
  newSession,
} from './support.mjs';

const reference = await loadReference();
const shaded = reference.animations.find((entry) => entry.name === 'shaded_gif');
const twoFrame = reference.encoded.find((entry) => entry.name === 'gif_first_frame');
const long = reference.longAnimation;

function previewOf(session, generation, frame) {
  const trailing = frame === undefined ? [] : [frame];
  session.startPreview(generation, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, ...trailing);
  drive(session, generation, MAX_SAFE);
  const rendered = session.finish(generation);
  const result = { width: rendered.width, height: rendered.height, pixels: rendered.takePixels() };
  rendered.free();
  return result;
}

function assertFrameParity(actual, expected, label) {
  assert.equal(actual.width, expected.previewWidth, label);
  assert.equal(actual.height, expected.previewHeight, label);
  assert.equal(Buffer.compare(actual.pixels, bytes(expected.preview)), 0, `${label}: preview differs from the native frame`);
}

function animatedSession(entry) {
  return ImageSession.fromEncoded(bytes(entry.bytes));
}

test('the shaded GIF imports as three frames with normalized delays and an endless loop', () => {
  const session = animatedSession(shaded);
  assert.equal(session.frameCount, shaded.frames.length);
  assert.equal(session.frameCount, 3);
  assert.deepEqual(Array.from(session.frameDelays), shaded.frames.map((frame) => frame.delay));
  assert.deepEqual(Array.from(session.frameDelays), [5, 10, 50]);
  assert.equal(session.loopCount, Infinity);
  assert.equal(session.sourceWidth, shaded.width);
  assert.equal(session.sourceHeight, shaded.height);
  session.dispose();
});

for (const [index, expected] of shaded.frames.entries()) {
  test(`frame ${index} preview matches its native composited frame`, () => {
    const session = animatedSession(shaded);
    assertFrameParity(previewOf(session, index, index), expected, `frame ${index}`);
    session.dispose();
  });
}

test('omitting the frame argument previews frame 0 exactly as before', () => {
  const session = animatedSession(shaded);
  assertFrameParity(previewOf(session, 0), shaded.frames[0], 'default frame');
  session.dispose();
});

test('the frames of the shaded GIF render distinct previews', () => {
  const session = animatedSession(shaded);
  const first = previewOf(session, 0, 0);
  const middle = previewOf(session, 1, 1);
  const last = previewOf(session, 2, 2);
  assert.notEqual(Buffer.compare(first.pixels, last.pixels), 0);
  assert.notEqual(Buffer.compare(middle.pixels, last.pixels), 0);
  session.dispose();
});

test('the two-frame GIF reports play-once and a 10 cs normalized first delay', () => {
  const session = animatedSession(twoFrame);
  assert.equal(session.frameCount, 2);
  assert.deepEqual(Array.from(session.frameDelays), [10, 10]);
  assert.equal(session.loopCount, 0);
  assertFrameParity(previewOf(session, 0, 0), twoFrame, 'gif_first_frame frame 0');
  session.dispose();
});

test('a GIF over the frame limit falls back to its first frame', () => {
  const session = animatedSession(long);
  assert.equal(session.frameCount, 1);
  assert.equal(session.frameDelays.length, 0);
  assert.equal(session.loopCount, 0);
  assertFrameParity(previewOf(session, 0), long, 'long animation first frame');
  assertCode(() => previewOf(session, 1, 1), 'invalid_argument', 'only frame 0 exists');
  session.dispose();
});

test('a still source has one frame, no delays and plays once', () => {
  const session = newSession(4, 4);
  assert.equal(session.frameCount, 1);
  assert.equal(session.frameDelays.length, 0);
  assert.equal(session.loopCount, 0);
  assertCode(() => previewOf(session, 0, 1), 'invalid_argument', 'still frame 1');
  session.dispose();
});

test('out-of-range and malformed frame indices are invalid arguments and keep the job state', () => {
  const session = animatedSession(shaded);
  for (const frame of [3, 99, -1, 1.5, Number.NaN]) {
    assertCode(
      () => session.startPreview(0, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, frame),
      'invalid_argument',
      `startPreview frame ${frame}`,
    );
  }
  assertCode(() => session.startOriginalPreview(0, 3), 'invalid_argument', 'original frame 3');
  assertCode(
    () => session.startExport(0, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, 'png', 3),
    'invalid_argument',
    'export frame 3',
  );
  assert.equal(session.stage(), 0, 'failed starts leave the session idle');
  assertFrameParity(previewOf(session, 0, 2), shaded.frames[2], 'valid start after failures');
  session.dispose();
});

test('an original preview of a chosen frame reaches Ready', () => {
  const session = animatedSession(shaded);
  session.startOriginalPreview(0, 1);
  drive(session, 0, MAX_SAFE);
  const rendered = session.finish(0);
  assert.ok(rendered.width > 0 && rendered.height > 0);
  rendered.free();
  session.dispose();
});

test('export frames produce PNG bytes that follow the frame index', () => {
  const session = animatedSession(shaded);
  const exportFrame = (generation, frame) => {
    session.startExport(generation, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, 'png', frame);
    drive(session, generation, MAX_SAFE);
    return Buffer.concat(session.finishExport(generation).map((chunk) => Buffer.from(chunk)));
  };
  const first = exportFrame(0, 0);
  const last = exportFrame(1, 2);
  assert.equal(first[0], 0x89);
  assert.equal(first.subarray(1, 4).toString('latin1'), 'PNG');
  assert.notEqual(Buffer.compare(first, last), 0);
  session.dispose();
});

test('frame accessors report disposal after dispose', () => {
  const session = animatedSession(shaded);
  session.dispose();
  for (const getter of ['frameCount', 'frameDelays', 'loopCount']) {
    assertCode(() => session[getter], 'disposed', getter);
  }
});

function skipSubBlocks(bytes, start) {
  let at = start;
  for (;;) {
    const length = bytes[at];
    at += 1;
    if (length === 0) return at;
    at += length;
  }
}

function gifFrameCount(bytes) {
  assert.equal(Buffer.from(bytes.subarray(0, 6)).toString('latin1'), 'GIF89a');
  let at = 13;
  if (bytes[10] & 0x80) at += 3 * (2 << (bytes[10] & 0x07));
  let frames = 0;
  for (;;) {
    const block = bytes[at];
    if (block === 0x21) {
      at = skipSubBlocks(bytes, at + 2);
    } else if (block === 0x2c) {
      frames += 1;
      const flags = bytes[at + 9];
      at += 10;
      if (flags & 0x80) at += 3 * (2 << (flags & 0x07));
      at = skipSubBlocks(bytes, at + 1);
    } else if (block === 0x3b) {
      assert.equal(at + 1, bytes.length, 'the trailer must end the stream');
      return frames;
    } else {
      assert.fail(`unexpected GIF block ${block}`);
    }
  }
}

function gifExport(session, generation, frame) {
  const trailing = frame === undefined ? [] : [frame];
  session.startExport(generation, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, 'gif', ...trailing);
  const chunks = [];
  while (session.stage() !== JobStage.Ready) {
    session.advance(generation, 4096);
    for (const chunk of session.drainExport(generation)) chunks.push(Buffer.from(chunk));
  }
  for (const chunk of session.finishExport(generation)) chunks.push(Buffer.from(chunk));
  return Buffer.concat(chunks);
}

test('the shaded animation exports as a GIF with one image per frame', () => {
  const started = animatedSession(shaded);
  started.startExport(1, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, 'gif');
  assert.equal(started.progressTotal(), started.frameCount, 'GIF progress counts frames');
  started.dispose();

  const exported = gifExport(animatedSession(shaded), 1);
  assert.equal(gifFrameCount(exported), shaded.frames.length);
  assert.equal(gifFrameCount(exported), 3);
  assert.ok(exported.includes(Buffer.from('NETSCAPE2.0', 'latin1')), 'an endless loop writes the NETSCAPE extension');
});

test('a GIF export refuses a still source and a nonzero frame with invalid_argument', () => {
  const still = newSession();
  assertCode(() => still.startExport(1, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, 'gif'), 'invalid_argument', 'still gif');
  still.dispose();

  const session = animatedSession(shaded);
  assertCode(() => session.startExport(1, 5, 10, 128, 'classic', undefined, undefined, undefined, undefined, undefined, undefined, undefined, 'gif', 1), 'invalid_argument', 'gif frame');
  session.dispose();
});
