import { plugin, type BunPlugin } from 'bun'
import { mock } from 'bun:test'
import { shallowRef } from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'

/**
 * Lets a test import a single-file component, as Vite's Vue plugin builds it for the
 * page: its script and template become one module, the template inlined into its
 * setup. Bun has no loader for `.vue` files of its own.
 *
 * Styles are left out: a test of happy-dom lays nothing out, and a test in Chromium
 * builds its page with this plugin and the stylesheet Tailwind makes.
 */
export const vueSingleFileComponents: BunPlugin = {
  name: 'vue single-file components',
  setup(build) {
    build.onLoad({ filter: /\.vue$/ }, async ({ path }) => {
      const parsed = parse(await Bun.file(path).text(), { filename: path })
      if (parsed.errors.length > 0)
        throw parsed.errors[0]
      // A component of a template alone, such as a divider, has no script
      // (the parser drops an empty one) to inline it into, which
      // compileScript requires: it is given one that does nothing.
      const { descriptor } = parsed.descriptor.script || parsed.descriptor.scriptSetup
        ? parsed
        : parse(`<script setup lang="ts">\nvoid 0\n</script>\n${parsed.descriptor.source}`, { filename: path })
      const id = Bun.hash(path).toString(16)
      const script = compileScript(descriptor, { id, inlineTemplate: true })
      return { contents: script.content, loader: script.lang === 'ts' ? 'ts' : 'js' }
    })
  },
}
plugin(vueSingleFileComponents)

/**
 * Builds a Chromium test's page without the modules that gather files with
 * Vite's `import.meta.glob`, which Bun lacks: the file icons' theme, as
 * below, and the vendors' logos; a file and a vendor then show no mark.
 */
export const withoutViteGlobs: BunPlugin = {
  name: 'without Vite globs',
  setup(build) {
    build.onLoad({ filter: /[/\\]files[/\\]file-icon-theme\.ts$/ }, () => ({
      contents: `import { shallowRef } from 'vue'
export const fileIconTheme = shallowRef(null)
export const ensureFileIconTheme = () => {}
export const fileIconUrl = async () => null`,
      loader: 'ts',
    }))
  },
}

// The file icons' theme gathers its glyphs with Vite's `import.meta.glob`,
// which Bun lacks: in a test there is no theme, and a file shows no icon.
void mock.module('@demicodes/web-ui/files/file-icon-theme', () => ({
  fileIconTheme: shallowRef(null),
  ensureFileIconTheme: () => {},
  fileIconUrl: async () => null,
}))
