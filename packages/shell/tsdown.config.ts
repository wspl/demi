import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/storage.ts', 'src/host-fs.ts', 'src/testing.ts', 'src/build.ts', 'src/bash.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
