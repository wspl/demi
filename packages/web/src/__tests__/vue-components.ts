import { plugin } from 'bun'
import { mock } from 'bun:test'
import { shallowRef } from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'

/**
 * Lets a test import a single-file component, as Vite's Vue plugin builds
 * it for the page: its script and template become one module, the template
 * inlined into its setup. Bun has no loader for `.vue` files of its own.
 * Styles are left out: a test of happy-dom lays nothing out.
 */
plugin({
  name: 'vue single-file components',
  setup(build) {
    build.onLoad({ filter: /\.vue$/ }, async ({ path }) => {
      const { descriptor, errors } = parse(await Bun.file(path).text(), { filename: path })
      if (errors.length > 0)
        throw errors[0]
      const id = Bun.hash(path).toString(16)
      const script = compileScript(descriptor, { id, inlineTemplate: true })
      return { contents: script.content, loader: script.lang === 'ts' ? 'ts' : 'js' }
    })
  },
})

// The file icons' theme gathers its glyphs with Vite's `import.meta.glob`,
// which Bun lacks: in a test there is no theme, and a file shows no icon.
void mock.module('@demicodes/web-ui/files/file-icon-theme', () => ({
  fileIconTheme: shallowRef(null),
  ensureFileIconTheme: () => {},
  fileIconUrl: async () => null,
}))
