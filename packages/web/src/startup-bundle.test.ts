import { expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { build } from 'vite'

// What a cold start downloads: the files the page's `index.html` loads hold no
// part of the code editor, its highlighter or its languages, nor KaTeX, its
// stylesheet and fonts, nor the terminal's xterm.js, each loaded on first use
// or, but xterm, in the background once the first conversation has shown.
// A static import from anywhere the first screen reaches would put them back
// silently, so the test builds the page as production does, without writing it,
// and reads the result. Cost: one bundle of the app, about 5 s, which no
// cheaper test can stand in for: only the bundler knows what a static import
// pulls in.

const root = resolve(import.meta.dir, '..')

/** The libraries that load on first use, each by the modules it is made of. */
const LAZY = {
  'code editor': /[\\/]@(codemirror|lezer)[\\/]/,
  KaTeX: /[\\/]katex[\\/]/,
  xterm: /[\\/]@xterm[\\/]/,
}

test('the files index.html loads hold no part of the code editor, KaTeX or xterm', async () => {
  const built = await build({
    root,
    configFile: resolve(root, 'vite.config.ts'),
    logLevel: 'silent',
    // The workspace's packages from their sources, as the tests read them.
    resolve: { conditions: ['development'] },
    build: { write: false, reportCompressedSize: false },
  })
  // A build that watches answers with its watcher; this one does not watch.
  if (!Array.isArray(built) && !('output' in built))
    throw new Error('the build watched')
  const outputs = Array.isArray(built) ? built : [built]
  const files = new Map(outputs.flatMap((output) => output.output).map((file) => [file.fileName, file]))
  const page = files.get('index.html')
  if (page?.type !== 'asset')
    throw new Error('the build emitted no index.html')
  const loaded = [...String(page.source).matchAll(/(?:src|href)="\/(assets\/[^"]+\.(?:js|css))"/g)].map((match) => match[1]!)
  expect(loaded.some((fileName) => fileName.endsWith('.css'))).toBe(true)
  /** The modules of `library` a chunk holds, and for a stylesheet whether it is KaTeX's, which names its fonts. */
  const parts = (fileName: string, library: keyof typeof LAZY) => {
    const file = files.get(fileName)
    if (file?.type === 'chunk')
      return file.moduleIds.filter((id) => LAZY[library].test(id))
    if (library === 'KaTeX' && file?.type === 'asset' && fileName.endsWith('.css'))
      return String(file.source).includes('KaTeX_Main') ? [fileName] : []
    return []
  }
  for (const library of Object.keys(LAZY) as (keyof typeof LAZY)[]) {
    expect({ library, parts: loaded.flatMap((fileName) => parts(fileName, library)) }).toEqual({ library, parts: [] })
    // The library is in the build still, in chunks of its own.
    expect({ library, built: [...files.keys()].some((fileName) => parts(fileName, library).length > 0) }).toEqual({ library, built: true })
  }
}, 60_000)
