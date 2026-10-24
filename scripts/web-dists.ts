// `vite build` of the web app reads the builds of the workspace packages
// `@demicodes/protocol`, `conversation-client` and `utils`.
//
// These packages export their sources under the `development` condition, which
// Vite's development server sets, and their `dist` under `import`, which
// `vite build` reads. `dist` is not committed, so a fresh checkout has none, and
// after a change to a package it holds the old exports: the build then fails
// with a missing export.
//
// This builds the packages whose inputs differ from the ones their `dist` was
// built from, and leaves the rest. A stamp inside `dist` records the inputs'
// digest, because file times say nothing here: `contracts`, which must have run
// before this, rewrites the generated sources on every run, changed or not.
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const packages = ['utils', 'protocol', 'conversation-client']
const stampName = '.built-from'

/** A digest of what `tsdown` builds a package from: its sources and its configuration. */
function digest(directory: string): string {
  const hasher = new Bun.CryptoHasher('sha256')
  const files = [...new Bun.Glob('src/**/*').scanSync(directory), 'package.json', 'tsdown.config.ts', 'tsconfig.json']
    .filter((file) => existsSync(join(directory, file)))
    .sort()
  for (const file of files) {
    hasher.update(`${file}\0`)
    hasher.update(readFileSync(join(directory, file)))
  }
  return hasher.digest('hex')
}

const repository = resolve(import.meta.dir, '..')
for (const name of packages) {
  const directory = join(repository, 'packages', name)
  const stamp = join(directory, 'dist', stampName)
  const inputs = digest(directory)
  if (existsSync(stamp) && readFileSync(stamp, 'utf8') === inputs) {
    continue
  }
  const built = Bun.spawnSync([process.execPath, 'run', 'tsdown'], { cwd: directory, stdout: 'pipe', stderr: 'pipe' })
  if (built.exitCode !== 0) {
    console.error(`tsdown failed in packages/${name}:\n${built.stdout.toString()}${built.stderr.toString()}`)
    process.exit(built.exitCode ?? 1)
  }
  writeFileSync(stamp, inputs)
  console.error(`Built packages/${name} for the web app's build`)
}
