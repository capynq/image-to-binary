import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

const frontendRoot = dirname(fileURLToPath(import.meta.url))

export default defineConfig({
  plugins: [react()],
  server: {
    host: 'localhost',
    fs: {
      strict: true,
      allow: [frontendRoot, resolve(frontendRoot, '../assets/fonts')],
    },
  },
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
  },
})
