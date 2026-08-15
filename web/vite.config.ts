import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  base: '/vic3-analyzer/',
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
