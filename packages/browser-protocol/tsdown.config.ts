import { defineConfig } from 'tsdown'

export default defineConfig({ entry: ['src/index.ts', 'src/live.ts'], format: ['esm'], dts: true, clean: true })
