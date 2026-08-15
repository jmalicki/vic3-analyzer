import '@testing-library/jest-dom/vitest'
import 'fake-indexeddb/auto'
import { webcrypto } from 'node:crypto'

Object.defineProperty(globalThis, 'crypto', {
  configurable: true,
  value: webcrypto,
})

if (!File.prototype.arrayBuffer) {
  File.prototype.arrayBuffer = function () {
    return new Response(this).arrayBuffer()
  }
}
