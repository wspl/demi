import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/testing.ts', 'src/credentials-pool.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
