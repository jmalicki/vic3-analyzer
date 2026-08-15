import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'

const packageJson = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
) as { version: string }
const revision = (() => {
  try {
    return execFileSync('git', ['rev-parse', '--short', 'HEAD'], {
      encoding: 'utf8',
    }).trim()
  } catch {
    return 'unknown'
  }
})()
const buildTime = new Date().toISOString()

// https://vite.dev/config/
export default defineConfig({
  base: '/vic3-analyzer/',
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
    __BUILD_TIME__: JSON.stringify(buildTime),
    __GIT_REVISION__: JSON.stringify(revision),
  },
  plugins: [react()],
  test: {
    fileParallelism: false,
    projects: [
      {
        extends: true,
        test: {
          name: 'unit',
          environment: 'jsdom',
          setupFiles: './src/test/setup.ts',
          include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
          exclude: ['src/**/*.wasm.test.ts'],
        },
      },
      {
        extends: true,
        test: {
          name: 'wasm',
          environment: 'node',
          include: ['src/**/*.wasm.test.ts'],
          // Real wasm needs the compiled package under public/wasm/.
          testTimeout: 30_000,
        },
      },
    ],
  },
})
