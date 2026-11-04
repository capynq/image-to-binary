import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
export const crateDir = join(root, 'crates', 'wasm');
export const outDir = join(root, '.local', 'wasm', 'pkg');
export const referencePath = join(root, '.local', 'wasm', 'reference.json');
export const toolsRoot = join(root, '.local', 'wasm-tools');
export const exe = process.platform === 'win32' ? '.exe' : '';
export const artifact = 'image_to_binary_wasm';

export function pinnedBindgenVersion() {
  const manifest = readFileSync(join(crateDir, 'Cargo.toml'), 'utf8');
  const match = /^wasm-bindgen\s*=\s*"=(\d+\.\d+\.\d+)"/m.exec(manifest);
  if (!match) {
    throw new Error('crates/wasm/Cargo.toml must pin wasm-bindgen as "=x.y.z"');
  }
  return match[1];
}

function pinnedToolchain() {
  const file = readFileSync(join(root, 'rust-toolchain.toml'), 'utf8');
  return /channel\s*=\s*"([^"]+)"/.exec(file)?.[1] ?? '';
}

/** Explicit CARGO_HOME/RUSTUP_HOME/PATH win; `.local` toolchains are appended fallbacks. */
export function toolchainEnv() {
  const env = { ...process.env };
  const local = join(root, '.local');
  if (!env.CARGO_HOME && existsSync(join(local, 'cargo'))) env.CARGO_HOME = join(local, 'cargo');
  if (!env.RUSTUP_HOME && existsSync(join(local, 'rustup'))) env.RUSTUP_HOME = join(local, 'rustup');
  env.RUSTUP_AUTO_INSTALL ??= '0';
  const toolchains = join(local, 'rustup', 'toolchains');
  const bins = existsSync(toolchains)
    ? readdirSync(toolchains)
        .filter((name) => name.startsWith(pinnedToolchain()))
        .map((name) => join(toolchains, name, 'bin'))
    : [];
  const fallbacks = [join(local, 'cargo', 'bin'), ...bins].filter(existsSync);
  const key = Object.keys(env).find((name) => name.toLowerCase() === 'path') ?? 'PATH';
  env[key] = [env[key], ...fallbacks].filter(Boolean).join(delimiter);
  return env;
}

export function run(command, args, { env = toolchainEnv(), cwd = root } = {}) {
  const result = spawnSync(command, args, { cwd, env, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} exited with status ${result.status}`);
  }
}

export function bindgenBinary() {
  if (process.env.WASM_BINDGEN_BIN) return process.env.WASM_BINDGEN_BIN;
  const local = join(toolsRoot, 'bin', `wasm-bindgen${exe}`);
  return existsSync(local) ? local : 'wasm-bindgen';
}

export function bindgenVersionMatches(binary, env = toolchainEnv()) {
  const result = spawnSync(binary, ['--version'], { env, encoding: 'utf8' });
  return !result.error && result.status === 0 && result.stdout.trim() === `wasm-bindgen ${pinnedBindgenVersion()}`;
}

export function main(task) {
  try {
    task();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
