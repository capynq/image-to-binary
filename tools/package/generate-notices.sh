#!/usr/bin/env bash
# Writes .local/notices/THIRD_PARTY_LICENSES.txt for one desktop package build.
#
# Usage: tools/package/generate-notices.sh <rust-target-triple>
# Inputs are read offline: cargo metadata (normal dependencies of the desktop crate only, filtered to
# the target) and npm ls of the frontend's production tree. Exit status 1 means a license outside the
# allowlist; 2 means a component that could not be resolved and needs manual review. Both fail the build.
# NOTICES_METADATA, NOTICES_NPM and NOTICES_OUTPUT override the inputs and output for tests.
set -euo pipefail

PACKAGE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$PACKAGE_DIR/../.." && pwd)"
DESKTOP_DIR="${PACKAGE_DESKTOP_DIR:-$REPO_ROOT/crates/desktop}"
FRONTEND_DIR="${PACKAGE_FRONTEND_DIR:-$REPO_ROOT/frontend}"
OUTPUT="${NOTICES_OUTPUT:-$REPO_ROOT/.local/notices/THIRD_PARTY_LICENSES.txt}"
CANONICAL_DIR="$PACKAGE_DIR/licenses"

[ "$#" -eq 1 ] || { echo "usage: generate-notices.sh <rust-target-triple>" >&2; exit 64; }
RUST_TARGET="$1"

WORK="$(mktemp -d "$REPO_ROOT/.local/notices-work.XXXXXX")"
cleanup() {
    find "$WORK" -depth -delete 2>/dev/null || true
    rmdir "$WORK" 2>/dev/null || true
}
trap cleanup EXIT

METADATA="${NOTICES_METADATA:-$WORK/metadata.json}"
NPM_TREE="${NOTICES_NPM:-$WORK/npm.json}"
if [ -z "${NOTICES_METADATA:-}" ]; then
    cargo metadata --offline --locked --format-version 1 \
        --manifest-path "$DESKTOP_DIR/Cargo.toml" --filter-platform "$RUST_TARGET" > "$METADATA"
fi
if [ -z "${NOTICES_NPM:-}" ]; then
    (cd "$FRONTEND_DIR" && npm ls --omit=dev --all --json --offline > "$NPM_TREE" 2>/dev/null) || true
fi

COMMIT="$(git -C "$REPO_ROOT" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)"
if [ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=no 2>/dev/null)" ]; then
    COMMIT="$COMMIT (uncommitted tracked changes present)"
fi

mkdir -p "$(dirname "$OUTPUT")"
STAGED="$WORK/THIRD_PARTY_LICENSES.txt"
set +e
NOTICE_METADATA="$METADATA" NOTICE_NPM="$NPM_TREE" NOTICE_CANONICAL="$CANONICAL_DIR" \
NOTICE_FRONTEND="$FRONTEND_DIR" NOTICE_TARGET="$RUST_TARGET" NOTICE_COMMIT="$COMMIT" \
NOTICE_OUT="$STAGED" node --input-type=module - <<'JS'
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const env = process.env;
const metadata = JSON.parse(fs.readFileSync(env.NOTICE_METADATA, 'utf8'));
const npmTree = JSON.parse(fs.readFileSync(env.NOTICE_NPM, 'utf8'));
const canonicalDir = env.NOTICE_CANONICAL;
const LICENSE_FILE = /^(LICEN[CS]E|COPYING|NOTICE|UNLICEN[CS]E)/i;

const LEAF_ALLOWED = (id) =>
  ['MIT', 'Apache-2.0', 'ISC', 'Zlib', 'Unicode-3.0', 'Unicode-DFS-2016', 'MPL-2.0', '0BSD', 'CC0-1.0', 'Unlicense', 'MIT-0'].includes(id) ||
  /^BSD-/.test(id);

function parseLicense(expression) {
  const tokens = expression.match(/\(|\)|\/|[A-Za-z0-9.+-]+/g) ?? [];
  let pos = 0;
  const peek = () => tokens[pos];
  const take = () => tokens[pos++];
  function atom() {
    if (peek() === '(') {
      take();
      const inner = orExpr();
      if (take() !== ')') throw new Error('unbalanced parentheses');
      return inner;
    }
    const id = take();
    if (id === undefined || /^(AND|OR|WITH)$/.test(id) || id === ')') throw new Error(`unexpected token ${id}`);
    if (peek() === 'WITH') {
      take();
      const exception = take();
      return { op: 'leaf', id: `${id} WITH ${exception}`, allowed: false };
    }
    return { op: 'leaf', id, allowed: LEAF_ALLOWED(id) };
  }
  function andExpr() {
    let node = atom();
    while (peek() === 'AND') {
      take();
      node = { op: 'and', left: node, right: atom() };
    }
    return node;
  }
  function orExpr() {
    let node = andExpr();
    while (peek() === 'OR' || peek() === '/') {
      take();
      node = { op: 'or', left: node, right: andExpr() };
    }
    return node;
  }
  const tree = orExpr();
  if (pos !== tokens.length) throw new Error('trailing tokens');
  return tree;
}

function evaluate(node) {
  if (node.op === 'leaf') return node.allowed;
  if (node.op === 'and') return evaluate(node.left) && evaluate(node.right);
  return evaluate(node.left) || evaluate(node.right);
}

function leaves(node, out = []) {
  if (node.op === 'leaf') out.push(node.id);
  else {
    leaves(node.left, out);
    leaves(node.right, out);
  }
  return out;
}

const canonicalFor = {
  MIT: 'MIT.txt',
  'Apache-2.0': 'Apache-2.0.txt',
  Zlib: 'Zlib.txt',
  'BSD-3-Clause': 'BSD-3-Clause.txt',
  'MPL-2.0': 'MPL-2.0.txt',
};

const texts = new Map();
function addText(label, content) {
  const hash = crypto.createHash('sha256').update(content).digest('hex').slice(0, 12);
  if (!texts.has(hash)) texts.set(hash, { label, content });
  return hash;
}

const problems = { violations: [], review: [] };
const components = [];
let canonicalCrates = 0;

const pkgById = new Map(metadata.packages.map((p) => [p.id, p]));
const nodeById = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
const reachable = new Set([metadata.resolve.root]);
const queue = [metadata.resolve.root];
while (queue.length) {
  const id = queue.shift();
  for (const dep of nodeById.get(id).deps) {
    if (!dep.dep_kinds.some((k) => k.kind === null)) continue;
    if (!reachable.has(dep.pkg)) {
      reachable.add(dep.pkg);
      queue.push(dep.pkg);
    }
  }
}

const firstParty = [];
for (const id of [...reachable].sort()) {
  if (id === metadata.resolve.root) continue;
  const pkg = pkgById.get(id);
  if (!pkg.source) {
    firstParty.push(pkg.name);
    continue;
  }
  const dir = path.dirname(pkg.manifest_path);
  const label = `${pkg.name} ${pkg.version}`;
  const holders = (pkg.authors ?? []).map((a) => a.replace(/\s*<[^>]*>/, '').trim()).filter(Boolean);
  const repository = pkg.repository ?? '';
  if (!pkg.license) {
    problems.review.push(`${label}: no license field in Cargo.toml`);
    components.push({ kind: 'crate', label, license: '(none)', holders, repository, refs: [] });
    continue;
  }
  let tree;
  try {
    tree = parseLicense(pkg.license);
  } catch (error) {
    problems.review.push(`${label}: unparseable license "${pkg.license}" (${error.message})`);
    components.push({ kind: 'crate', label, license: pkg.license, holders, repository, refs: [] });
    continue;
  }
  if (!evaluate(tree)) {
    problems.violations.push(`${label}: license "${pkg.license}" is outside the allowlist`);
  }
  const files = fs
    .readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && fs.statSync(path.join(dir, f)).isFile())
    .sort();
  const refs = [];
  let usesCanonical = false;
  if (files.length) {
    for (const file of files) {
      refs.push(addText(`${label} ${file}`, fs.readFileSync(path.join(dir, file), 'utf8')));
    }
  } else {
    canonicalCrates += 1;
    usesCanonical = true;
    for (const id of new Set(leaves(tree))) {
      const canonical = canonicalFor[id];
      if (!canonical) {
        problems.review.push(`${label}: no license file and no canonical text for "${id}"`);
        continue;
      }
      refs.push(addText(`canonical ${id}`, fs.readFileSync(path.join(canonicalDir, canonical), 'utf8')));
    }
  }
  components.push({ kind: 'crate', label, license: pkg.license, holders, repository, refs, canonical: usesCanonical });
}

const npmSeen = new Map();
const npmRoot = env.NOTICE_FRONTEND;
function findInstall(name, startDir) {
  let dir = startDir;
  for (;;) {
    const candidate = path.join(dir, 'node_modules', name);
    if (fs.existsSync(path.join(candidate, 'package.json'))) return candidate;
    if (dir === npmRoot || path.dirname(dir) === dir) return null;
    dir = path.dirname(dir);
  }
}
function walkNpm(node, startDir) {
  for (const [name, child] of Object.entries(node.dependencies ?? {})) {
    const dir = findInstall(name, startDir) ?? path.join(npmRoot, 'node_modules', name);
    const key = `${name}@${child.version}`;
    if (!npmSeen.has(key)) {
      npmSeen.set(key, { name, version: child.version, dir });
    }
    walkNpm(child, dir);
  }
}
walkNpm(npmTree, npmRoot);

for (const { name, version, dir } of [...npmSeen.values()].sort((a, b) => a.name.localeCompare(b.name))) {
  const label = `${name} ${version} (npm)`;
  let manifest = {};
  try {
    manifest = JSON.parse(fs.readFileSync(path.join(dir, 'package.json'), 'utf8'));
  } catch {
    problems.review.push(`${label}: package.json not found at ${dir}`);
    continue;
  }
  const license = manifest.license ?? (manifest.licenses ?? []).map((l) => l.type ?? l).join(' OR ');
  if (!license) {
    problems.review.push(`${label}: no license field`);
    continue;
  }
  let tree;
  try {
    tree = parseLicense(String(license));
  } catch (error) {
    problems.review.push(`${label}: unparseable license "${license}"`);
    continue;
  }
  if (!evaluate(tree)) problems.violations.push(`${label}: license "${license}" is outside the allowlist`);
  const files = fs
    .readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && fs.statSync(path.join(dir, f)).isFile())
    .sort();
  if (!files.length) problems.review.push(`${label}: no license file in package`);
  const refs = files.map((file) =>
    addText(`${label} ${file}`, fs.readFileSync(path.join(dir, file), 'utf8')),
  );
  const holders = manifest.author ? [typeof manifest.author === 'string' ? manifest.author.replace(/\s*<[^>]*>/, '').trim() : manifest.author.name ?? ''] : [];
  components.push({ kind: 'npm', label, license: String(license), holders, repository: manifest.repository?.url ?? manifest.repository ?? '', refs });
}

if (problems.violations.length) {
  for (const line of problems.violations) console.error(`notices: violation: ${line}`);
  process.exit(1);
}
if (problems.review.length) {
  for (const line of problems.review) console.error(`notices: manual review: ${line}`);
  process.exit(2);
}

const crateCount = components.filter((c) => c.kind === 'crate').length;
const npmCount = components.filter((c) => c.kind === 'npm').length;
const lines = [];
lines.push('Third-party licenses for the Image to Binary desktop package');
lines.push('');
lines.push('The application itself is licensed under GPL-3.0-only (see licenses/LICENSE).');
lines.push(`Corresponding source: this repository at commit ${env.NOTICE_COMMIT}.`);
lines.push('MPL-2.0 covered crates: their source is available from crates.io (registry.crates.io).');
lines.push(`Rust target: ${env.NOTICE_TARGET}`);
lines.push(`Rust crates shipped (normal dependencies): ${crateCount}`);
lines.push(`Crates without a license file that use canonical texts from tools/package/licenses: ${canonicalCrates}`);
lines.push(`Frontend npm packages shipped (production): ${npmCount}`);
lines.push(`First-party path crates (GPL-3.0-only, not listed): ${firstParty.join(', ')}`);
lines.push('');
lines.push('Components');
lines.push('==========');
for (const c of components) {
  lines.push('');
  lines.push(`${c.label}`);
  lines.push(`  source: ${c.kind === 'npm' ? 'npm' : 'crates.io'}${c.repository ? ` (${c.repository})` : ''}`);
  lines.push(`  license: ${c.license}`);
  lines.push(`  copyright holders (from ${c.kind === 'npm' ? 'package.json' : 'Cargo.toml authors'}): ${c.holders.length ? c.holders.join(', ') : '(not stated in metadata)'}`);
  lines.push(`  repository (from metadata): ${c.repository || '(not stated in metadata)'}`);
  if (c.canonical) {
    lines.push(`  license texts: ${c.refs.map((r) => `[${r}]`).join(' ')} (canonical template from tools/package/licenses; the holders above apply)`);
  } else {
    lines.push(`  license texts: ${c.refs.length ? c.refs.map((r) => `[${r}]`).join(' ') : '(none)'}`);
  }
}
lines.push('');
lines.push('License texts');
lines.push('=============');
for (const [hash, { label, content }] of [...texts.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  lines.push('');
  lines.push(`[${hash}] from ${label}`);
  lines.push('-'.repeat(72));
  lines.push(content.replace(/\s+$/, ''));
}
lines.push('');
fs.writeFileSync(env.NOTICE_OUT, lines.join('\n'));
console.log(`notices: ${crateCount} crates, ${npmCount} npm packages, ${texts.size} distinct license texts`);
JS
NODE_STATUS=$?
set -e
if [ "$NODE_STATUS" -ne 0 ]; then
    echo "generate-notices: failed with status $NODE_STATUS; no notices file was written" >&2
    exit "$NODE_STATUS"
fi
[ -s "$STAGED" ] || { echo "generate-notices: staged notices file is empty" >&2; exit 1; }
mv "$STAGED" "$OUTPUT"
echo "generate-notices: wrote $OUTPUT"
