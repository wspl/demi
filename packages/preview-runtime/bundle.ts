// Bundles the preview runtime into one script (`builds-and-releases.md`
// § Preview runtime). `bun xtask preview-runtime` runs it once it has
// generated the rewriter's WebAssembly and its glue into src/generated/:
// `bun bundle.ts <output file> [--debug]`. `--debug` keeps names and adds an
// inline source map, for reading stacks in a page.
import { build } from 'esbuild'

const [output, ...flags] = Bun.argv.slice(2)
if (!output) throw new Error('usage: bun bundle.ts <output file> [--debug]')
const debug = flags.includes('--debug')

const result = await build({
  entryPoints: [`${import.meta.dir}/src/runtime.js`],
  bundle: true,
  write: false,
  platform: 'browser',
  format: 'iife',
  minify: !debug,
  sourcemap: debug ? 'inline' : false,
  // The rewriter's module is embedded as bytes, which the runtime compiles
  // synchronously (src/rewriter.js).
  loader: { '.wasm': 'binary' },
})
const [bundle] = result.outputFiles
if (!bundle) throw new Error('esbuild wrote no bundle')
// The runtime keeps its own function, so that it can install itself in a
// realm the page creates without an address (a blank frame), from its source
// text (src/frame-runtime.js).
await Bun.write(
  output,
  `(function demiRuntime(){if(!globalThis.__proxyRuntimeFunction)Object.defineProperty(globalThis,'__proxyRuntimeFunction',{value:demiRuntime});${bundle.text}\n})();\n`,
)
