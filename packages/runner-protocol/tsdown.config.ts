import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/codec.ts', 'src/local.ts', 'src/release.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
