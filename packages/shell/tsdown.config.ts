import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/storage.ts', 'src/testing.ts', 'src/build.ts', 'src/hostless.ts', 'src/node.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
