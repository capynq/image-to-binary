import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

const frontendRoot = dirname(fileURLToPath(import.meta.url))
const wasmPackage = resolve(frontendRoot, '../.local/wasm/pkg')
const heifDist = resolve(frontendRoot, '../.local/wasm-codecs/dist')
const heifGlueBuilt = resolve(heifDist, 'heif_decode.mjs')
const heifGlueDisabled = process.env.IMAGE_TO_BINARY_DISABLE_HEIF_GLUE === '1'
const heifGlueAvailable =
  !heifGlueDisabled &&
  existsSync(heifGlueBuilt) &&
  existsSync(resolve(heifDist, 'heif_decode.wasm'))
const heifGlue = heifGlueAvailable
  ? heifGlueBuilt
  : resolve(frontendRoot, 'src/processing/heif/unavailableGlue.ts')

export default defineConfig({
  plugins: [react()],
  worker: {
    format: 'es',
  },
  resolve: {
    alias: [{ find: 'heif-decoder-glue', replacement: heifGlue }],
  },
  build: {
    assetsInlineLimit: 0,
  },
  server: {
    host: 'localhost',
    fs: {
      strict: true,
      allow: [frontendRoot, resolve(frontendRoot, '../assets/fonts'), wasmPackage, heifDist],
    },
  },
  test: {
    include: ['src/**/*.test.{ts,tsx}'],
    environment: 'node',
  },
})
