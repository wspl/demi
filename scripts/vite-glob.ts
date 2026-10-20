// Vite's eager `import.meta.glob` for `bun test`, which Bun lacks.
//
// - A call with `eager: true` and `import: 'default'` becomes one static
//   import per matching file, keyed by its path as Vite keys it
//   ('./logos/openai.svg'), so a test loads the same modules a build does.
// - A file's URL in a test is its path, as Bun answers a static import of
//   an asset.
// - A lazy call is left as it is, and a module that makes one is mocked
//   where a test reaches it
//   (`packages/web-ui/src/__tests__/vue-components.ts`).
import { plugin } from 'bun'
import { dirname } from 'node:path'

/** A call: its pattern, then its options' braces, which hold no braces. */
const CALL = /import\.meta\.glob(?:<[^>]*>)?\(\s*(['"])(\.\/[^'"]+)\1\s*,\s*\{([^{}]*)\}\s*\)/g

void plugin({
  name: 'vite-eager-glob',
  setup(build) {
    build.onLoad({ filter: /\/packages\/[^/]+\/src\/.+\.ts$/ }, async ({ path }) => {
      const source = await Bun.file(path).text()
      // A runtime plugin cannot hand a file back to Bun's own loader: an
      // answer without contents is taken for a failed mock.
      if (!source.includes('import.meta.glob'))
        return { contents: source, loader: 'ts' }
      const imports: string[] = []
      const contents = source.replace(CALL, (call, _quote, pattern: string, options: string) => {
        if (!/\beager:\s*true\b/.test(options) || !/\bimport:\s*['"]default['"]/.test(options))
          return call
        const files = [...new Bun.Glob(pattern.slice(2)).scanSync({ cwd: dirname(path) })].sort()
        const entries = files.map((file) => {
          const name = `__glob${imports.length}`
          imports.push(`import ${name} from ${JSON.stringify(`./${file}`)}`)
          return `${JSON.stringify(`./${file}`)}: ${name}`
        })
        return `({ ${entries.join(', ')} })`
      })
      return { contents: `${imports.join('\n')}\n${contents}`, loader: 'ts' }
    })
  },
})
