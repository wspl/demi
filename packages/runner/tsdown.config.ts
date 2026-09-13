import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/testing.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
