import { expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { build } from 'vite'

// What a cold start downloads (`file-previews.md` § Code): the files the
// page's `index.html` loads hold no part of the code editor, its highlighter
// or its languages, which load on first use. A static import from anywhere
// the first screen reaches would put them back silently, so the test builds
// the page as production does, without writing it, and reads the result.
// Cost: one bundle of the app, about 5 s, which no cheaper test can stand in
// for: only the bundler knows what a static import pulls in.

const root = resolve(import.meta.dir, '..')

test('the files index.html loads hold no part of the code editor, its highlighter or its languages', async () => {
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
  const loaded = [...String(page.source).matchAll(/(?:src|href)="\/(assets\/[^"]+\.js)"/g)].map((match) => match[1]!)
  const editorModules = (fileName: string) => {
    const file = files.get(fileName)
    return file?.type === 'chunk' ? file.moduleIds.filter((id) => /[\\/]@(codemirror|lezer)[\\/]/.test(id)) : []
  }
  expect(loaded.length).toBeGreaterThan(0)
  expect(loaded.flatMap(editorModules)).toEqual([])
  // The editor is in the build still, in chunks of its own.
  expect([...files.keys()].some((fileName) => editorModules(fileName).length > 0)).toBe(true)
}, 60_000)
