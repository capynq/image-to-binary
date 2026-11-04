import { existsSync, mkdirSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';

import { artifact, crateDir, main, outDir, referencePath, run, toolchainEnv } from './common.mjs';

main(() => {
  const env = toolchainEnv();
  if (!existsSync(join(outDir, `${artifact}.js`))) {
    throw new Error('generated package missing; run: node crates/wasm/scripts/build.mjs');
  }
  mkdirSync(dirname(referencePath), { recursive: true });
  run('cargo', ['run', '--package', 'image-to-binary-processing', '--example', 'reference_fixtures', '--locked', '--', referencePath], { env });
  const testsDir = join(crateDir, 'tests', 'node');
  const files = readdirSync(testsDir)
    .filter((name) => name.endsWith('.test.mjs'))
    .sort()
    .map((name) => join(testsDir, name));
  run(process.execPath, ['--test', ...files], {
    env: { ...env, IMAGE_TO_BINARY_WASM_PKG: outDir, IMAGE_TO_BINARY_REFERENCE: referencePath },
  });
});
