// The product's and the gallery's style sheets generate the utility classes
// the plugin packages use: Tailwind writes only the classes it finds in the
// sources a sheet names, so a plugin's component whose package the sheet does
// not scan shows unstyled, as the live view's hidden text field did with its
// caret showing over the picture. The check compiles each sheet with the
// Tailwind its Vite plugin runs, as a build does.
import { expect, test } from 'bun:test'
import { readFileSync, readdirSync, realpathSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'

const root = resolve(import.meta.dir, '../..')
// The compiler and the scanner the product's `@tailwindcss/vite` runs, which only it depends on.
const plugin = dirname(realpathSync(Bun.resolveSync('@tailwindcss/vite', join(root, 'packages/web'))))
const { compile } = await import(Bun.resolveSync('@tailwindcss/node', plugin))
const { Scanner } = await import(Bun.resolveSync('@tailwindcss/oxide', plugin))

/** Every component of every plugin package. */
function pluginComponents(): string[] {
  const packages = readdirSync(join(root, 'packages')).filter((name) => name.startsWith('plugin-'))
  return packages.flatMap((name) => {
    const source = join(root, 'packages', name, 'src')
    return readdirSync(source, { recursive: true, encoding: 'utf8' })
      .filter((file) => file.endsWith('.vue'))
      .map((file) => join(source, file))
  })
}

for (const sheet of ['packages/web/src/style.css', 'packages/web-gallery/src/style.css']) {
  test(`${sheet} generates the plugin packages' classes`, async () => {
    const file = join(root, sheet)
    const compiler = await compile(readFileSync(file, 'utf8'), { base: dirname(file), onDependency: () => {} })
    const scanner = new Scanner({ sources: compiler.sources })
    const css: string = compiler.build(scanner.scan())
    const scanned = new Set(scanner.files.map((path: string) => realpathSync(path)))
    const components = pluginComponents()
    expect(components.length).toBeGreaterThan(0)
    expect(components.filter((component) => !scanned.has(realpathSync(component)))).toEqual([])
    // The live view's hidden text field, a class only the browser plugin uses.
    expect(css).toContain('.caret-transparent')
  })
}
