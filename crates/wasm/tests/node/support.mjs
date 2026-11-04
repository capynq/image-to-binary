import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..', '..', '..', '..');
export const packageDir = process.env.IMAGE_TO_BINARY_WASM_PKG ?? join(repoRoot, '.local', 'wasm', 'pkg');
export const referencePath = process.env.IMAGE_TO_BINARY_REFERENCE ?? join(repoRoot, '.local', 'wasm', 'reference.json');

export const bindings = await import(pathToFileURL(join(packageDir, 'image_to_binary_wasm.js')).href);
export const instance = await bindings.default({
  module_or_path: await readFile(join(packageDir, 'image_to_binary_wasm_bg.wasm')),
});
export const { ImageSession, JobStage } = bindings;
export const MAX_SAFE = Number.MAX_SAFE_INTEGER;

export function assertCode(fn, code, label = '') {
  assert.throws(fn, (error) => {
    assert.ok(error instanceof Error, `${label}: expected an Error`);
    assert.ok(!(error instanceof WebAssembly.RuntimeError), `${label}: must not be a trap`);
    assert.equal(error.name, 'ImageToBinaryError', label);
    assert.equal(error.code, code, `${label}: ${error.message}`);
    return true;
  });
}

export function gradient(width, height) {
  const pixels = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const offset = (y * width + x) * 4;
      pixels[offset] = (x * 9 + y * 3) % 256;
      pixels[offset + 1] = (x * 2 + y * 13) % 256;
      pixels[offset + 2] = (x * 7 + y * 5) % 256;
      pixels[offset + 3] = 255;
    }
  }
  return pixels;
}

export function newSession(width = 30, height = 30) {
  return ImageSession.fromRgba(width, height, gradient(width, height));
}

export function startClassic(session, generation, threshold = 128) {
  return session.startPreview(generation, 5, 10, threshold, 'classic');
}

export function drive(session, generation, budget) {
  const stages = [session.stage()];
  while (stages.at(-1) !== JobStage.Ready) {
    const previous = stages.at(-1);
    const before = session.progressCompleted();
    const total = session.progressTotal();
    const next = session.advance(generation, budget);
    const completed = session.progressCompleted();
    if (next === previous) {
      assert.equal(completed, Math.min(before + budget, total), 'a stage spends exactly the budget');
    } else if (next === JobStage.Ready) {
      assert.equal(completed, session.progressTotal());
      assert.ok(before + budget >= total);
    } else {
      assert.equal(completed, 0, 'a newly entered stage spends no budget');
      assert.ok(before + budget >= total);
    }
    if (next !== previous) stages.push(next);
  }
  return stages;
}

export async function loadReference() {
  return JSON.parse(await readFile(referencePath, 'utf8'));
}

export function bytes(base64) {
  return new Uint8Array(Buffer.from(base64, 'base64'));
}
