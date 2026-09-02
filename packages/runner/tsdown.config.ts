import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/entry.ts', 'src/testing.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
  // The tinyjs API is provided by the runtime, never bundled.
  external: [/^tinyjs:/],
})
