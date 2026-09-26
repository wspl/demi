// The TypeScript package boundary check (`crates-and-packages.md` § Boundary
// checks): the npm workspace, its manifests and its production sources
// against the document's TypeScript graph, which the check reads rather than
// a copy of it.
import { expect, test } from 'bun:test'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { builtinModules } from 'node:module'
import { dirname, join, resolve, sep } from 'node:path'
import { parseSync } from 'oxc-parser'
import { z } from 'zod'

const root = resolve(import.meta.dir, '../..')
const SCOPE = '@demicodes/'

const dependencyTable = z.record(z.string(), z.string())
const exportTarget = z.union([z.string(), z.record(z.string(), z.string())])
const packageManifest = z.object({
  name: z.string().startsWith(SCOPE),
  dependencies: dependencyTable.optional(),
  devDependencies: dependencyTable.optional(),
  peerDependencies: dependencyTable.optional(),
  optionalDependencies: dependencyTable.optional(),
  exports: z.record(z.string(), exportTarget).optional(),
})
const rootManifest = z.object({
  workspaces: z.array(z.string()),
  scripts: z.object({ test: z.string() }),
})
const rootTsconfig = z.object({
  compilerOptions: z.object({ paths: z.record(z.string(), z.array(z.string())) }),
})

interface WorkspacePackage {
  /** The name without the `@demicodes/` scope, as the graph writes it. */
  name: string
  /** The package's directory, relative to the repository root. */
  directory: string
  manifest: z.infer<typeof packageManifest>
}

function readJson<T>(path: string, schema: z.ZodType<T>): T {
  return schema.parse(JSON.parse(readFileSync(join(root, path), 'utf8')))
}

/** The `text` block under the document's TypeScript graph heading, as name -> dependencies. */
function readGraph(): Map<string, string[]> {
  const document = readFileSync(join(root, 'docs/architecture/crates-and-packages.md'), 'utf8')
  const heading = document.indexOf('\n### TypeScript packages\n')
  const open = document.indexOf('```text\n', heading)
  const close = document.indexOf('\n```', open)
  if (heading < 0 || open < 0 || close < 0) throw new Error('the document has no TypeScript graph block')
  const graph = new Map<string, string[]>()
  for (const line of document.slice(open + '```text\n'.length, close).split('\n')) {
    const match = /^([a-z][a-z0-9-]*) -> (none|[a-z][a-z0-9-]*(?:, [a-z][a-z0-9-]*)*)$/.exec(line)
    if (!match) throw new Error(`not a graph line: ${JSON.stringify(line)}`)
    const [, name, dependencies] = match
    if (graph.has(name)) throw new Error(`${name} has two lines`)
    graph.set(name, dependencies === 'none' ? [] : dependencies.split(', '))
  }
  return graph
}

const graph = readGraph()
const workspace = readJson('package.json', rootManifest)
const packages: WorkspacePackage[] = workspace.workspaces.map((directory) => {
  const manifest = readJson(join(directory, 'package.json'), packageManifest)
  return { name: manifest.name.slice(SCOPE.length), directory, manifest }
})

/** Every `.ts` file under `directory`, depth first. */
function typeScriptFiles(directory: string): string[] {
  const files: string[] = []
  for (const entry of readdirSync(join(root, directory), { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory() && entry.name !== 'node_modules') files.push(...typeScriptFiles(path))
    else if (entry.isFile() && entry.name.endsWith('.ts')) files.push(path)
  }
  return files
}

function isTest(path: string): boolean {
  return path.split(sep).includes('__tests__') || path.endsWith('.test.ts')
}

/** The production `.ts` sources of a package: everything under `src` but tests. */
function productionSources(pkg: WorkspacePackage): string[] {
  return typeScriptFiles(join(pkg.directory, 'src')).filter((path) => !isTest(path))
}

/**
 * The module specifiers a file names: imports and re-exports, type-only ones
 * included, and dynamic imports of a literal. A computed dynamic import names
 * nothing a check could read; the production sources have none.
 */
function specifiers(path: string): string[] {
  const source = readFileSync(join(root, path), 'utf8')
  const parsed = parseSync(path, source)
  if (parsed.errors.length > 0) throw new Error(`${path}: ${parsed.errors.map((error) => error.message).join('; ')}`)
  const named = parsed.module.staticImports.map((entry) => entry.moduleRequest.value)
  for (const statement of parsed.module.staticExports) {
    for (const entry of statement.entries) {
      if (entry.moduleRequest) named.push(entry.moduleRequest.value)
    }
  }
  for (const entry of parsed.module.dynamicImports) {
    const literal = /^(['"`])([^'"`$]*)\1$/.exec(source.slice(entry.moduleRequest.start, entry.moduleRequest.end))
    if (literal) named.push(literal[2])
  }
  return named
}

/** The workspace package a scoped specifier names, without the scope. */
function scopedPackage(specifier: string): string | null {
  return specifier.startsWith(SCOPE) ? specifier.slice(SCOPE.length).split('/')[0] : null
}

test('every workspace package has exactly one line, and every line names a package', () => {
  const names = packages.map((pkg) => pkg.name)
  expect(names.filter((name, index) => names.indexOf(name) !== index)).toEqual([])
  expect([...names].sort()).toEqual([...graph.keys()].sort())
  const unknown = [...graph].flatMap(([name, dependencies]) =>
    dependencies.filter((dependency) => !graph.has(dependency)).map((dependency) => `${name} -> ${dependency}`))
  expect(unknown).toEqual([])
})

test('each manifest declares exactly its line as dependencies, never elsewhere', () => {
  const violations: string[] = []
  for (const pkg of packages) {
    const declared = Object.keys(pkg.manifest.dependencies ?? {}).map(scopedPackage).filter((name) => name !== null)
    if (declared.sort().join(', ') !== [...(graph.get(pkg.name) ?? [])].sort().join(', '))
      violations.push(`${pkg.name} declares [${declared.join(', ')}], its line [${graph.get(pkg.name)?.join(', ')}]`)
    for (const table of ['devDependencies', 'peerDependencies', 'optionalDependencies'] as const) {
      for (const dependency of Object.keys(pkg.manifest[table] ?? {})) {
        if (scopedPackage(dependency) !== null) violations.push(`${pkg.name} declares ${dependency} in ${table}`)
      }
    }
  }
  expect(violations).toEqual([])
})

test('production imports stay within the importing package\'s line', () => {
  const violations: string[] = []
  for (const pkg of packages) {
    const allowed = new Set([pkg.name, ...(graph.get(pkg.name) ?? [])])
    const home = join(root, pkg.directory) + sep
    for (const path of productionSources(pkg)) {
      for (const specifier of specifiers(path)) {
        const named = scopedPackage(specifier)
        if (named !== null && !allowed.has(named)) violations.push(`${path} imports ${specifier}`)
        if (specifier.startsWith('.') && !resolve(root, dirname(path), specifier).startsWith(home))
          violations.push(`${path} reaches outside its package: ${specifier}`)
      }
    }
  }
  expect(violations).toEqual([])
})

test('no production source imports a Node built-in', () => {
  const builtins = new Set(builtinModules)
  const violations: string[] = []
  for (const pkg of packages) {
    for (const path of productionSources(pkg)) {
      for (const specifier of specifiers(path)) {
        if (specifier.startsWith('node:') || builtins.has(specifier.split('/')[0]))
          violations.push(`${path} imports ${specifier}`)
      }
    }
  }
  expect(violations).toEqual([])
})

test('built exports name their sources, and the root paths mirror them', () => {
  const { paths } = readJson('tsconfig.json', rootTsconfig).compilerOptions
  const violations: string[] = []
  // Every root entry resolves through the one wildcard, so it must be `src/index.ts`.
  if (paths[`${SCOPE}*`]?.join() !== './packages/*/src/index.ts')
    violations.push(`tsconfig.json: ${SCOPE}* must map to ./packages/*/src/index.ts`)
  for (const pkg of packages) {
    for (const [entry, target] of Object.entries(pkg.manifest.exports ?? {})) {
      // A string target is source the consumer compiles (web-ui's `./*`).
      if (typeof target === 'string') continue
      const source = target.development
      if (!source || !existsSync(join(root, pkg.directory, source))) {
        violations.push(`${pkg.name} ${entry}: no development source`)
        continue
      }
      const file = `./${join(pkg.directory, source)}`
      if (entry === '.') {
        if (file !== `./${pkg.directory}/src/index.ts`) violations.push(`${pkg.name}: its root entry is ${file}, not src/index.ts`)
      } else if (paths[`${SCOPE}${pkg.name}${entry.slice(1)}`]?.join() !== file) {
        violations.push(`tsconfig.json: ${SCOPE}${pkg.name}${entry.slice(1)} must map to ${file}`)
      }
    }
  }
  for (const [alias, files] of Object.entries(paths)) {
    if (alias.includes('*')) continue
    for (const file of files) {
      if (!existsSync(join(root, file))) violations.push(`tsconfig.json: ${alias} names ${file}, which does not exist`)
    }
  }
  expect(violations).toEqual([])
})

test('the test script names every package with tests, and only paths that exist', () => {
  const named = workspace.scripts.test.split(/\s+/).filter((token) => token.startsWith('./'))
  const violations = named.filter((path) => !existsSync(join(root, path))).map((path) => `${path} does not exist`)
  for (const pkg of packages) {
    const tested = typeScriptFiles(join(pkg.directory, 'src')).some((path) => path.endsWith('.test.ts'))
    if (tested && !named.includes(`./${pkg.directory}/src`)) violations.push(`${pkg.name}'s tests are not in the test script`)
  }
  expect(violations).toEqual([])
})

test('the graph is acyclic', () => {
  const done = new Set<string>()
  const visiting: string[] = []
  const cycles: string[] = []
  const visit = (name: string): void => {
    if (done.has(name)) return
    if (visiting.includes(name)) {
      cycles.push([...visiting.slice(visiting.indexOf(name)), name].join(' -> '))
      return
    }
    visiting.push(name)
    for (const dependency of graph.get(name) ?? []) visit(dependency)
    visiting.pop()
    done.add(name)
  }
  for (const name of graph.keys()) visit(name)
  expect(cycles).toEqual([])
})
