import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/entry.ts', 'src/testing.ts', 'src/serve/index.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
  // The txiki.js API is provided by the runtime, never bundled.
  external: [/^tjs:/],
})
