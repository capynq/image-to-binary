import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

import {
  artifact,
  bindgenBinary,
  bindgenVersionMatches,
  main,
  outDir,
  pinnedBindgenVersion,
  root,
  run,
  toolchainEnv,
} from './common.mjs';

main(() => {
  const env = toolchainEnv();
  const binary = bindgenBinary();
  if (!bindgenVersionMatches(binary, env)) {
    throw new Error(
      `wasm-bindgen ${pinnedBindgenVersion()} is required; run: node crates/wasm/scripts/setup.mjs`,
    );
  }
  run('cargo', ['build', '--package', 'image-to-binary-wasm', '--target', 'wasm32-unknown-unknown', '--release', '--locked'], { env });
  const targetDir = env.CARGO_TARGET_DIR ? resolve(root, env.CARGO_TARGET_DIR) : join(root, 'target');
  const wasm = join(targetDir, 'wasm32-unknown-unknown', 'release', `${artifact}.wasm`);
  mkdirSync(outDir, { recursive: true });
  run(binary, ['--target', 'web', '--out-dir', outDir, '--out-name', artifact, '--ts-typed-array-buffers', wasm], { env });
  const metadata = { name: 'image-to-binary-wasm', private: true, type: 'module', main: `${artifact}.js`, types: `${artifact}.d.ts` };
  writeFileSync(join(outDir, 'package.json'), `${JSON.stringify(metadata, null, 2)}\n`);
});
