import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/main.ts'],
  format: ['esm'],
  external: ['bun:ffi'],
  dts: true,
  clean: true,
})
