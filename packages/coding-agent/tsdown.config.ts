import { commandModulesAsText } from '@demicodes/shell/build'
import { defineConfig } from 'tsdown'

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
  plugins: [commandModulesAsText()],
})
