import { existsSync } from 'node:fs';
import { join } from 'node:path';

import { bindgenVersionMatches, exe, main, pinnedBindgenVersion, run, toolsRoot } from './common.mjs';

main(() => {
  const version = pinnedBindgenVersion();
  const binary = join(toolsRoot, 'bin', `wasm-bindgen${exe}`);
  if (existsSync(binary) && bindgenVersionMatches(binary)) {
    console.log(`wasm-bindgen ${version} already installed in ${toolsRoot}`);
    return;
  }
  run('cargo', ['install', '--locked', '--root', toolsRoot, '--version', version, 'wasm-bindgen-cli']);
  if (!bindgenVersionMatches(binary)) {
    throw new Error(`installed wasm-bindgen does not report version ${version}`);
  }
});
