import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/node.ts', 'src/testing.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
