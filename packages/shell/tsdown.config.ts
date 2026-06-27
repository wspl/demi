import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/storage.ts', 'src/host-fs.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
