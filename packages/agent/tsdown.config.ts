import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts', 'src/client-entry.ts', 'src/stdio-transport.ts', 'src/command-bridge.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
})
