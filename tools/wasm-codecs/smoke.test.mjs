/*
 * Smoke tests for the Emscripten HEIC/AVIF glue. Run after tools/wasm-codecs/build.sh with
 *   node --test tools/wasm-codecs/smoke.test.mjs
 * Hostile-input cases patch in-memory copies of the native fixtures (ispe dimensions, pitm item
 * id); no fixture file is modified.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile, access } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "../..");
const distModule = path.join(repoRoot, ".local/wasm-codecs/dist/heif_decode.mjs");
const fixtureDir = path.join(repoRoot, ".local/native-codecs/fixtures");

const WIDTH = 64;
const HEIGHT = 48;
const PIXEL_BYTES = WIDTH * HEIGHT * 4;
const STATUS = { ok: 0, unsupported: 1, invalid: 2, limit: 3, noMemory: 4 };
const TOLERANCE = 8;

const EXPECTED_FIRST_PIXEL = {
    "sample.heic": [0, 2, 121, 255],
    "sample.avif": [1, 0, 129, 255],
};

async function loadModule() {
    try {
        await access(distModule);
    } catch {
        throw new Error(`missing ${distModule}; run tools/wasm-codecs/build.sh first`);
    }
    const { default: createHeifDecoderModule } = await import(pathToFileURL(distModule).href);
    return createHeifDecoderModule();
}

function readMessage(mod) {
    const start = mod._hwc_message();
    const heap = mod.HEAPU8;
    let end = start;
    while (end < heap.length && heap[end] !== 0) {
        end += 1;
    }
    return new TextDecoder().decode(heap.subarray(start, end));
}

function decode(mod, bytes, maxPixels) {
    const ptr = mod._malloc(Math.max(bytes.length, 1));
    try {
        mod.HEAPU8.set(bytes, ptr);
        const status = mod._hwc_decode(ptr, bytes.length, maxPixels);
        const message = readMessage(mod);
        if (status !== STATUS.ok) {
            mod._hwc_release();
            return { status, message, width: 0, height: 0, rgba: null };
        }
        const width = mod._hwc_width();
        const height = mod._hwc_height();
        const pixels = mod._hwc_pixels();
        const rgba = mod.HEAPU8.slice(pixels, pixels + width * height * 4);
        mod._hwc_release();
        return { status, message, width, height, rgba };
    } finally {
        mod._free(ptr);
    }
}

async function fixture(name) {
    return new Uint8Array(await readFile(path.join(fixtureDir, name)));
}

/** Rewrites every ispe box, which carries the declared width and height of each item. */
function patchIspe(bytes, width, height) {
    const out = Buffer.from(bytes);
    let patched = 0;
    for (let i = 4; i + 16 <= out.length; i += 1) {
        if (out.toString("latin1", i, i + 4) === "ispe") {
            out.writeUInt32BE(width, i + 8);
            out.writeUInt32BE(height, i + 12);
            patched += 1;
        }
    }
    assert.ok(patched > 0, "fixture has an ispe box");
    return new Uint8Array(out);
}

/** Points the primary item at an id that no item carries (version 0 pitm, 16-bit id). */
function patchPrimaryItemId(bytes, itemId) {
    const out = Buffer.from(bytes);
    const at = out.indexOf("pitm", 4, "latin1");
    assert.ok(at >= 4, "fixture has a pitm box");
    assert.equal(out[at + 4], 0, "pitm version 0");
    out.writeUInt16BE(itemId, at + 8);
    return new Uint8Array(out);
}

function assertNear(actual, expected, tolerance, label) {
    for (let i = 0; i < expected.length; i += 1) {
        assert.ok(
            Math.abs(actual[i] - expected[i]) <= tolerance,
            `${label} channel ${i}: got ${actual[i]}, expected ${expected[i]} +/- ${tolerance}`,
        );
    }
}

test("HEIC and AVIF fixtures decode to 64x48 RGBA near the known gradient", async (t) => {
    const mod = await loadModule();
    for (const name of Object.keys(EXPECTED_FIRST_PIXEL)) {
        const result = decode(mod, await fixture(name), 50_000_000);
        assert.equal(result.status, STATUS.ok, `${name}: ${result.message}`);
        assert.equal(result.width, WIDTH, `${name} width`);
        assert.equal(result.height, HEIGHT, `${name} height`);
        assert.equal(result.rgba.byteLength, PIXEL_BYTES, `${name} byte length`);
        assertNear(result.rgba.subarray(0, 4), EXPECTED_FIRST_PIXEL[name], TOLERANCE, name);
        t.diagnostic(`${name} first pixel RGBA: ${Array.from(result.rgba.subarray(0, 4)).join(",")}`);
    }
});

test("pixel limit below width*height is rejected with status 3 and no pixels", async () => {
    const mod = await loadModule();
    const result = decode(mod, await fixture("sample.heic"), WIDTH * HEIGHT - 1);
    assert.equal(result.status, STATUS.limit);
    assert.match(result.message, /pixel limit/);
    assert.equal(result.rgba, null);
});

test("pixel limit equal to width*height is accepted", async () => {
    const mod = await loadModule();
    const result = decode(mod, await fixture("sample.avif"), WIDTH * HEIGHT);
    assert.equal(result.status, STATUS.ok, result.message);
});

test("truncated input returns a non-zero status with a message", async () => {
    const mod = await loadModule();
    const full = await fixture("sample.heic");
    for (const cut of [full.length >> 1, 64, 1]) {
        const result = decode(mod, full.subarray(0, cut), 50_000_000);
        assert.notEqual(result.status, STATUS.ok, `truncated at ${cut} bytes decoded unexpectedly`);
        assert.ok(result.message.length > 0, `truncated at ${cut} bytes has no message`);
        assert.equal(result.rgba, null);
    }
});

test("garbage input returns a non-zero status with a message", async () => {
    const mod = await loadModule();
    const garbage = Uint8Array.from({ length: 4096 }, (_, i) => (i * 131 + 7) & 0xff);
    const result = decode(mod, garbage, 50_000_000);
    assert.equal(result.status, STATUS.invalid);
    assert.ok(result.message.length > 0);
    const empty = decode(mod, new Uint8Array(0), 50_000_000);
    assert.equal(empty.status, STATUS.invalid);
});

test("repeated decode keeps HEAPU8 length bounded", async (t) => {
    const mod = await loadModule();
    const heic = await fixture("sample.heic");
    const avif = await fixture("sample.avif");
    const before = mod.HEAPU8.length;
    decode(mod, heic, 50_000_000);
    const afterWarmup = mod.HEAPU8.length;
    for (let i = 0; i < 50; i += 1) {
        const result = decode(mod, i % 2 === 0 ? heic : avif, 50_000_000);
        assert.equal(result.status, STATUS.ok, `iteration ${i}: ${result.message}`);
    }
    const after = mod.HEAPU8.length;
    t.diagnostic(`HEAPU8.length before=${before} afterWarmup=${afterWarmup} after50=${after}`);
    assert.ok(after - afterWarmup <= 16 * 1024 * 1024, `heap grew by ${after - afterWarmup} bytes over 50 decodes`);
});

test("declared dimensions above the pixel limit in a small file are refused before decoding", async (t) => {
    const mod = await loadModule();
    const bytes = patchIspe(await fixture("sample.heic"), 65535, 65535);
    const result = decode(mod, bytes, 50_000_000);
    assert.equal(result.status, STATUS.limit, result.message);
    assert.match(result.message, /pixel limit/);
    assert.equal(result.rgba, null);
    t.diagnostic(`declared 65535x65535 message: ${result.message}`);
});

test("a small declared size with a larger coded image hits the libheif ceiling, not the output", async (t) => {
    const mod = await loadModule();
    const bytes = patchIspe(await fixture("sample.avif"), 8, 8);
    const result = decode(mod, bytes, 1000);
    assert.equal(result.status, STATUS.limit, result.message);
    assert.equal(result.rgba, null);
    t.diagnostic(`small ispe, coded 64x48, max 1000 message: ${result.message}`);
});

test("a primary item that no item carries is refused as malformed input", async (t) => {
    const mod = await loadModule();
    const bytes = patchPrimaryItemId(await fixture("sample.heic"), 0xffff);
    const result = decode(mod, bytes, 50_000_000);
    assert.equal(result.status, STATUS.invalid, result.message);
    assert.ok(result.message.length > 0);
    assert.equal(result.rgba, null);
    t.diagnostic(`missing primary item message: ${result.message}`);
});

test("a declared size that disagrees with the coded picture is refused as malformed input", async (t) => {
    const mod = await loadModule();
    const bytes = patchIspe(await fixture("sample.avif"), 63, 47);
    const result = decode(mod, bytes, 50_000_000);
    assert.equal(result.status, STATUS.invalid, result.message);
    assert.equal(result.rgba, null);
    t.diagnostic(`mismatched ispe message: ${result.message}`);
});

const oddFixtureDir = process.env.WASM_ODD_FIXTURES ?? null;

test(
    "odd-sized HEIC and AVIF decode to exactly the declared 63x47 size",
    {
        skip:
            oddFixtureDir === null
                ? "set WASM_ODD_FIXTURES to a directory holding odd63x47.heic and odd63x47.avif (sips -z 47 63, then sips -s format heic|avif)"
                : false,
    },
    async (t) => {
        const mod = await loadModule();
        for (const name of ["odd63x47.heic", "odd63x47.avif"]) {
            const bytes = new Uint8Array(await readFile(path.join(oddFixtureDir, name)));
            const result = decode(mod, bytes, 50_000_000);
            assert.equal(result.status, STATUS.ok, `${name}: ${result.message}`);
            assert.equal(result.width, 63, `${name} width`);
            assert.equal(result.height, 47, `${name} height`);
            assert.equal(result.rgba.byteLength, 63 * 47 * 4, `${name} byte length`);
            t.diagnostic(`${name} first pixel RGBA: ${Array.from(result.rgba.subarray(0, 4)).join(",")}`);
        }
    },
);
