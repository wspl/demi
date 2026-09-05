import { expect, test } from 'bun:test'
import { readFile, readdir, stat } from 'node:fs/promises'
import { dirname, join, resolve, sep } from 'node:path'
import { parseSync, Visitor } from 'oxc-parser'

const repoRoot = resolve(import.meta.dir, '../../../..')

// What exists and what it exports comes from the workspace manifests; what may
// depend on what comes from `docs/package-boundaries.md` § Production Dependency
// Graph. Both are read here so the guard and its documents cannot drift apart.
const rootManifest = await readPackageManifest(resolveRepoPath('package.json'))
const workspaces = await readWorkspacePackages(rootManifest)
const manifests = new Map([...workspaces].map(([name, pkg]) => [name, pkg.manifest]))
const documentedDependencyGraph = await readDocumentedDependencyGraph()

// `web-ui` and `web` are Vite/Vue packages (`.vue` + `.ts`): the `.ts`-only source scans
// below do not cover them, and their boundary is enforced at the manifest level.
const browserPackages = new Set(['@demicodes/web-ui', '@demicodes/web'])
const productionPackageDirectories = new Map(
  [...workspaces].filter(([name]) => !browserPackages.has(name)).map(([name, pkg]) => [name, pkg.directory] as const),
)

// Every entry a package exports under its `development` condition, as the specifier
// production code writes (`@demicodes/<pkg>` or `@demicodes/<pkg>/<subpath>`) → the
// repo-relative source file. Wildcard exports (web-ui's source paths) are not entries.
const workspaceEntryFiles = new Map<string, string>()
for (const [name, pkg] of workspaces) {
  for (const [subpath, file] of pkg.developmentExports) {
    workspaceEntryFiles.set(subpath === '.' ? name : `${name}${subpath.slice(1)}`, `${pkg.directory}/${file.replace(/^\.\//, '')}`)
  }
}

function entryFile(specifier: string): string {
  const file = workspaceEntryFiles.get(specifier)
  if (!file) throw new Error(`${specifier} is not a development export of any workspace package`)
  return file
}

// The packages whose root entry runs on every runtime (`docs/package-boundaries.md`, each
// registry entry's `Entries`): no Node builtin anywhere in the static closure of the root.
const platformNeutralEntries = [
  '@demicodes/utils',
  '@demicodes/core',
  '@demicodes/provider',
  '@demicodes/agent',
  '@demicodes/shell',
  '@demicodes/coding-agent',
  '@demicodes/runner-protocol',
  '@demicodes/host-virtual',
  '@demicodes/command-loader',
  '@demicodes/tinybash',
  '@demicodes/host-remote',
] as const

// Entries that are Node adapters by design; a neutral root must not reach them.
const nodeOnlySubpaths = new Set(['@demicodes/agent/stdio'])

const forbiddenSourcePatterns = [
  ['node builtin import', /\b(?:import|export)\s+(?:type\s+)?(?:[^'"]*?\s+from\s+)?['"]node:/],
  ['node builtin require', /\brequire\(\s*['"]node:/],
  ['Buffer global', /\bBuffer\b/],
  ['process env/cwd', /\bprocess\.(?:env|cwd)\b/],
] as const

const neutralPackageLeakPatterns = [
  ['concrete provider package reference', /@demicodes\/provider-(?:claude-code|codex|openai-api|anthropic-api|grok-build|google)\b|provider-(?:claude-code|codex|openai-api|anthropic-api|grok-build|google)/i],
  ['concrete provider implementation class', /\b(?:ClaudeCodeProvider|CodexProvider|OpenAIApiProvider|AnthropicApiProvider|GrokBuildProvider|GoogleProvider)\b/],
  ['concrete catalog source label', /\bcodex-backend\b/i],
  ['provider backend identifier', /\b(?:backend-api|chatgpt\.com|api\.openai\.com|cli-chat-proxy\.grok\.com|generativelanguage\.googleapis\.com|responses_websockets)\b/i],
  ['concrete provider product name', /\bClaude Code\b|\bOpenAI Codex\b|\bGrok Build\b|\bGoogle Gemini\b/i],
] as const

const staticSpecifierPattern = /\b(?:import|export)\s+(?:type\s+)?(?:[^'"]*?\s+from\s+)?['"]([^'"]+)['"]/g
const dynamicSpecifierPattern = /\bimport\(\s*['"]([^'"]+)['"]\s*\)/g

const nodeOnlyFiles = new Set([...nodeOnlySubpaths].map((specifier) => resolveRepoPath(entryFile(specifier))))

for (const entryName of platformNeutralEntries) {
  test(`${entryName} root entry has no Node-only source in its static closure`, async () => {
    const violations = await findPlatformViolations(resolveRepoPath(entryFile(entryName)))

    expect(violations).toEqual([])
  })
}

test('only AgentServer imports AgentSession as a runtime value outside tests', async () => {
  const files = await listProductionSourceFiles()
  const violations: string[] = []

  for (const file of files) {
    const relativeFile = formatPath(file)
    const source = await readFile(file, 'utf8')
    if (!hasRuntimeImportFromAgent(source, 'AgentSession')) continue
    if (relativeFile !== 'packages/agent/src/node/assemble.ts') violations.push(relativeFile)
  }

  expect(violations).toEqual([])
})

test('runtime source uses the forked bash package without embedded upstream snapshots', async () => {
  const forbiddenDirs = [
    'packages/bash',
    'packages/shell/vendor',
  ]
  const existingForbiddenDirs: string[] = []
  for (const directory of forbiddenDirs) {
    if (await isDirectory(resolveRepoPath(directory))) existingForbiddenDirs.push(directory)
  }

  expect(existingForbiddenDirs).toEqual([])
})

test('@demicodes/shell does not depend on the agent runtime', async () => {
  const files = await listSourceFiles(resolveRepoPath('packages/shell/src'))
  const violations: string[] = []

  for (const file of files) {
    const source = await readFile(file, 'utf8')
    if (hasRuntimeImportFromPackage(source, '@demicodes/agent')) violations.push(formatPath(file))
  }

  expect(violations).toEqual([])
})

test('package manifests preserve layering boundaries', () => {
  expect(packageDependencyNames(manifests.get('@demicodes/shell')).filter((name) => name === '@demicodes/core' || name === '@demicodes/provider')).toEqual([])

  const platformNeutralPackages = [
    '@demicodes/utils',
    '@demicodes/core',
    '@demicodes/provider',
    '@demicodes/agent',
    '@demicodes/shell',
    '@demicodes/coding-agent',
  ]
  for (const packageName of platformNeutralPackages) {
    // Hosts are the product's to inject; tests may run against one.
    expect(productionDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/host-remote')
    expect(productionDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/host-virtual')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/provider-claude-code')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/provider-codex')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/provider-openai-api')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/provider-anthropic-api')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/provider-grok-build')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demicodes/provider-google')
  }

  const claudeProviderDependencies = packageDependencyNames(manifests.get('@demicodes/provider-claude-code'))
  expect(claudeProviderDependencies.some((name) => name === '@anthropic-ai/claude-agent-sdk' || name.includes('claude-agent-sdk'))).toBe(false)

  const webUiDependencies = packageDependencyNames(manifests.get('@demicodes/web-ui'))
  for (const forbidden of [
    '@demicodes/host-remote',
    '@demicodes/shell',
    '@demicodes/coding-agent',
    '@demicodes/provider-claude-code',
    '@demicodes/provider-codex',
    '@demicodes/provider-openai-api',
    '@demicodes/provider-anthropic-api',
    '@demicodes/provider-grok-build',
    '@demicodes/provider-google',
    '@demicodes/web',
  ]) {
    expect(webUiDependencies).not.toContain(forbidden)
  }
})

test('@demicodes/core and @demicodes/provider contain no concrete provider product details', async () => {
  const scopes = [
    ['@demicodes/core', 'packages/core/src'],
    ['@demicodes/provider', 'packages/provider/src'],
  ] as const
  const violations: string[] = []

  for (const [packageName, directory] of scopes) {
    const files = await listSourceFiles(resolveRepoPath(directory))
    for (const file of files) {
      const source = await readFile(file, 'utf8')
      for (const label of findNeutralPackageLeaks(file, source)) {
        violations.push(`${packageName}: ${formatPath(file)} contains ${label}`)
      }
    }
  }

  expect(violations.sort()).toEqual([])
})

test('models.dev client facts belong only to the provider catalog client', () => {
  const source = `export const url = 'https://models.dev/api.json';
    export const warning = 'Using stale models.dev catalog';`
  expect(findNeutralPackageLeaks(resolveRepoPath('packages/provider/src/models-dev.ts'), source)).toEqual([])
  for (const file of ['packages/provider/src/types.ts', 'packages/core/src/index.ts']) {
    expect(findNeutralPackageLeaks(resolveRepoPath(file), source)).toContain('models.dev client outside its owning module')
  }
})

test.each(['codex-backend', 'models.dev', 'cache'])('shared catalog types cannot expose the %s source label', (label) => {
  for (const file of ['packages/provider/src/types.ts', 'packages/provider/src/models-dev.ts', 'packages/core/src/index.ts']) {
    for (const source of [
      `export interface ProviderModelList { source: '${label}' }`,
      `export type CatalogSource = '${label}' | 'other'`,
      `export enum CatalogSource { Remote = '${label}' }`,
    ]) {
      expect(findNeutralPackageLeaks(resolveRepoPath(file), source)).toContain('concrete catalog source label in metadata')
    }
  }
})

test('generic quota cache metadata is independent of model catalog source labels', () => {
  const source = `export type ProviderQuotaSource = 'probe' | 'cache';
    export const quota = { source: 'cache' };`
  expect(findNeutralPackageLeaks(resolveRepoPath('packages/provider/src/quota.ts'), source)).toEqual([])
})

test('the documented production dependency graph is the workspace manifests\' graph', () => {
  expect([...documentedDependencyGraph.keys()].sort()).toEqual([...workspaces.keys()].sort())

  const violations: string[] = []
  for (const [name, pkg] of workspaces) {
    const declared = productionDependencyNames(pkg.manifest).filter((dependency) => dependency.startsWith('@demicodes/'))
    const documented = documentedDependencyGraph.get(name) ?? []
    if (declared.join(', ') !== documented.join(', ')) {
      violations.push(`${name} declares [${declared.join(', ')}] but docs/package-boundaries.md says [${documented.join(', ')}]`)
    }
  }

  expect(violations).toEqual([])
})

test('production source dependency graph follows documented package boundaries', async () => {
  const edges = await collectProductionWorkspaceImportEdges()
  const violations: string[] = []

  for (const edge of edges) {
    const allowed = documentedDependencyGraph.get(edge.fromPackage) ?? []
    if (!allowed.includes(edge.toPackage)) violations.push(`${edge.file} imports ${edge.specifier} (${edge.fromPackage} -> ${edge.toPackage})`)
  }

  expect([...new Set(violations)].sort()).toEqual([])
  expect(findPackageDependencyCycle(edges)).toBeNull()
})

test('production workspace imports are declared as package dependencies', async () => {
  const edges = await collectProductionWorkspaceImportEdges()
  const violations: string[] = []

  for (const edge of edges) {
    const manifest = manifests.get(edge.fromPackage)
    if (manifest?.dependencies?.[edge.toPackage]) continue
    violations.push(`${edge.fromPackage} imports ${edge.toPackage} in ${edge.file} but does not declare it in dependencies`)
  }

  expect([...new Set(violations)].sort()).toEqual([])
})

test('root tsconfig paths are the packages\' development exports', async () => {
  const tsconfig = JSON.parse(await readFile(resolveRepoPath('tsconfig.json'), 'utf8')) as { compilerOptions: { paths: Record<string, string[]> } }
  const { '@demicodes/*': roots, ...subpaths } = tsconfig.compilerOptions.paths

  // Roots resolve through the wildcard, so every package's root entry is its `src/index.ts`;
  // every other entry has its own alias, and no alias names an entry that does not exist.
  expect(roots).toEqual(['./packages/*/src/index.ts'])
  const expected: Record<string, string[]> = {}
  for (const [specifier, file] of workspaceEntryFiles) {
    const pkg = workspaces.get(specifier)
    if (pkg) expect(file).toBe(`${pkg.directory}/src/index.ts`)
    else expected[specifier] = [`./${file}`]
  }
  expect(sortedEntries(subpaths)).toEqual(sortedEntries(expected))
})

test('root scripts name existing paths and the test script covers every package with tests', async () => {
  const missing: string[] = []
  for (const [script, command] of Object.entries(rootManifest.scripts ?? {})) {
    for (const path of command.match(/packages\/[\w./-]+/g) ?? []) {
      if (!(await isFile(resolveRepoPath(path))) && !(await isDirectory(resolveRepoPath(path)))) missing.push(`${script}: ${path}`)
    }
  }
  expect(missing).toEqual([])

  const tested = new Set(rootManifest.scripts?.test?.match(/packages\/[\w-]+\/src/g) ?? [])
  const untested: string[] = []
  for (const [name, pkg] of workspaces) {
    if (!tested.has(`${pkg.directory}/src`) && (await hasTestFiles(resolveRepoPath(`${pkg.directory}/src`)))) untested.push(name)
  }
  expect(untested).toEqual([])
})

test('generic helpers provided by shared packages are not re-implemented in production source', async () => {
  // Helpers consolidated into a shared package. Re-defining one (instead of importing it) is a
  // code-reuse regression and must fail, the same way boundary violations do. `messageOf` is the
  // deleted alias of `errorMessage` and must not return. Each helper records the package that owns
  // it; the canonical definition lives under `home` and is exempt.
  const utilsHelperNames = [
    'isRecord',
    'numberOrZero',
    'asError',
    'errorMessage',
    'messageOf',
    'isAbortError',
    'AbortError',
    'throwIfAborted',
    'abortable',
    'noop',
    'encodeUtf8',
    'decodeUtf8',
    'concatBytes',
    'shortHash',
    'normalizePath',
    'dirnamePath',
    'isAbsolutePath',
    'normalizeBaseUrl',
    'parseJsonObject',
    'parseJsonOrString',
    'stringOrNull',
    'nonEmptyString',
    'numberOrNull',
  ]
  const sharedHelpers = [
    ...utilsHelperNames.map((name) => ({ name, home: 'packages/utils/', pkg: '@demicodes/utils' })),
    // zeroUsage returns core's TokenUsage, so its canonical home is @demicodes/core (utils cannot depend on core).
    { name: 'zeroUsage', home: 'packages/core/', pkg: '@demicodes/core' },
    // httpErrorCode is identical across the HTTP providers; its home is @demicodes/provider. (redactSecretText /
    // normalizeErrorCode / providerErrorFromUnknown are NOT banned: codex ships intentionally different variants.)
    { name: 'httpErrorCode', home: 'packages/provider/', pkg: '@demicodes/provider' },
  ]
  const files = await listProductionSourceFiles()
  const violations: string[] = []

  for (const file of files) {
    const relativePath = formatPath(file)
    const source = await readFile(file, 'utf8')
    for (const { name, home, pkg } of sharedHelpers) {
      if (relativePath.startsWith(home)) continue // the canonical definition lives here
      // Match a function or class definition (not local variables that happen to share the name).
      const definition = new RegExp(`\\b(?:export\\s+)?(?:async\\s+)?function ${name}\\b|\\b(?:export\\s+)?(?:abstract\\s+)?class ${name}\\b`)
      if (definition.test(source)) violations.push(`${relativePath} re-implements "${name}" (import it from ${pkg})`)
    }
  }

  expect(violations.sort()).toEqual([])
})

test('public root exports do not expose provider internals or testing helpers', async () => {
  const checks = [
    [
      'packages/provider/src/index.ts',
      [
        ['testing helper export', /\bexport\b[\s\S]*?from\s+['"]\.\/(?:stub|testing)['"]/],
      ],
    ],
    [
      'packages/provider-claude-code/src/index.ts',
      [
        ['wildcard export', /\bexport\s+\*\s+from\b/],
        ['internal module export', /['"]\.\/(?:cli|jsonl|output|transport)['"]/],
        ['provider class export', /\bClaudeCodeProvider\b/],
        ['catalog parser or test helper export', /\b(?:ModelCatalogFetch|modelsDevAnthropicCatalogToModelList|parseClaudeModelVersion|resetClaudeCodeModelCatalogCacheForTests)\b/],
      ],
    ],
    [
      'packages/provider-codex/src/index.ts',
      [
        ['wildcard export', /\bexport\s+\*\s+from\b/],
        ['internal module export', /['"]\.\/(?:responses|sse|transport)['"]/],
        ['provider class export', /\bCodexProvider\b/],
        ['auth store or transport helper export', /\b(?:FileCodexAuthStore\b|StaticCodexAuthStore\b|CodexAuthStore\b|buildCodexHeaders|responsesUrlForAuth)\b/],
        ['catalog parser or test helper export', /\b(?:ModelCatalogFetch|codexBackendModelsToModelList|resetCodexModelCatalogCacheForTests)\b/],
      ],
    ],
    [
      'packages/provider-openai-api/src/index.ts',
      [
        ['wildcard export', /\bexport\s+\*\s+from\b/],
        ['provider class export', /\bOpenAIApiProvider\b/],
      ],
    ],
    [
      'packages/provider-anthropic-api/src/index.ts',
      [
        ['wildcard export', /\bexport\s+\*\s+from\b/],
        ['provider class export', /\bAnthropicApiProvider\b/],
      ],
    ],
    [
      'packages/provider-grok-build/src/index.ts',
      [
        ['wildcard export', /\bexport\s+\*\s+from\b/],
        ['internal module export', /['"]\.\/(?:chat|headers)['"]/],
        ['provider class export', /\bGrokBuildProvider\b/],
        ['auth store helper export', /\b(?:FileGrokAuthStore\b|StaticGrokAuthStore\b|GrokAuthStore\b|buildGrokBuildHeaders)\b/],
      ],
    ],
    [
      'packages/provider-google/src/index.ts',
      [
        ['wildcard export', /\bexport\s+\*\s+from\b/],
        ['provider class export', /\bGoogleProvider\b/],
      ],
    ],
  ] as const
  const violations: string[] = []

  for (const [path, rules] of checks) {
    const source = await readFile(resolveRepoPath(path), 'utf8')
    for (const [label, pattern] of rules) {
      if (pattern.test(source)) violations.push(`${path} exposes ${label}`)
    }
  }

  expect(violations.sort()).toEqual([])
})

async function findPlatformViolations(entry: string): Promise<string[]> {
  const seen = new Set<string>()
  const pending = [entry]
  const violations: string[] = []

  while (pending.length > 0) {
    const file = pending.pop()
    if (!file || seen.has(file)) continue
    seen.add(file)

    const relativeFile = formatPath(file)
    if (nodeOnlyFiles.has(file)) violations.push(`${relativeFile} is an explicit Node-only adapter`)

    const source = await readFile(file, 'utf8')
    for (const [label, pattern] of forbiddenSourcePatterns) {
      if (pattern.test(source)) violations.push(`${relativeFile} contains ${label}`)
    }

    for (const specifier of findModuleSpecifiers(source)) {
      if (nodeOnlySubpaths.has(specifier)) {
        violations.push(`${relativeFile} imports explicit Node-only subpath ${specifier}`)
        continue
      }

      const resolved = await resolveImport(file, specifier)
      if (resolved) pending.push(resolved)
    }
  }

  return violations.sort()
}

function findModuleSpecifiers(source: string): string[] {
  const specifiers = new Set<string>()
  for (const pattern of [staticSpecifierPattern, dynamicSpecifierPattern]) {
    pattern.lastIndex = 0
    let match = pattern.exec(source)
    while (match) {
      const specifier = match[1]
      if (specifier) specifiers.add(specifier)
      match = pattern.exec(source)
    }
  }
  return [...specifiers]
}

function findNeutralPackageLeaks(file: string, source: string): string[] {
  const parsed = parseSync(formatPath(file), source, { sourceType: 'module' })
  if (parsed.errors.length > 0) {
    const messages = parsed.errors.map((error) => error.message).join('; ')
    throw new Error(`Unable to parse ${formatPath(file)} for boundary checks: ${messages}`)
  }

  const strings: string[] = []
  collectAstStrings(parsed.program, strings)
  const semanticSource = strings.join('\n')
  const violations = neutralPackageLeakPatterns
    .filter(([, pattern]) => pattern.test(semanticSource))
    .map(([label]) => label as string)
  if (formatPath(file) !== 'packages/provider/src/models-dev.ts' && /\bmodels\.dev\b/i.test(semanticSource)) {
    violations.push('models.dev client outside its owning module')
  }

  // Client URLs and diagnostics are implementation facts; source tags are not shared metadata.
  const metadataStrings: string[] = []
  const catalogTypeStrings: string[] = []
  new Visitor({
    TSLiteralType(node) { collectAstStrings(node, metadataStrings) },
    TSEnumMember(node) { collectAstStrings(node.initializer, metadataStrings) },
    TSInterfaceDeclaration(node) {
      if (/Model|Catalog/.test(node.id.name)) collectAstStrings(node, catalogTypeStrings)
    },
    TSTypeAliasDeclaration(node) {
      if (/Model|Catalog/.test(node.id.name)) collectAstStrings(node, catalogTypeStrings)
    },
    TSEnumDeclaration(node) {
      if (/Model|Catalog/.test(node.id.name)) collectAstStrings(node, catalogTypeStrings)
    },
    Property(node) {
      const key = node.key.type === 'Identifier' ? node.key.name : node.key.type === 'Literal' ? node.key.value : null
      if (key === 'source') collectAstStrings(node.value, metadataStrings)
    },
  }).visit(parsed.program)
  if (metadataStrings.some((value) => /^(?:codex-backend|models\.dev)$/i.test(value)) ||
    catalogTypeStrings.some((value) => /^(?:codex-backend|models\.dev|cache)$/i.test(value))) {
    violations.push('concrete catalog source label in metadata')
  }
  return violations
}

function collectAstStrings(value: unknown, output: string[]): void {
  if (typeof value === 'string') {
    output.push(value)
    return
  }
  if (value === null || typeof value !== 'object') return
  if (Array.isArray(value)) {
    for (const item of value) collectAstStrings(item, output)
    return
  }

  for (const [key, child] of Object.entries(value)) {
    if (key === 'type' || key === 'start' || key === 'end') continue
    collectAstStrings(child, output)
  }
}

async function resolveImport(fromFile: string, specifier: string): Promise<string | null> {
  if (specifier.startsWith('.')) return resolveLocalModule(dirname(fromFile), specifier)

  const workspaceEntry = workspaceEntryFiles.get(specifier)
  if (workspaceEntry) return resolveRepoPath(workspaceEntry)

  return null
}

async function resolveLocalModule(fromDir: string, specifier: string): Promise<string> {
  const base = resolve(fromDir, specifier)
  const candidates = [
    base,
    `${base}.ts`,
    `${base}.tsx`,
    `${base}.js`,
    base.endsWith('.js') ? `${base.slice(0, -3)}.ts` : '',
    join(base, 'index.ts'),
  ].filter(Boolean)

  for (const candidate of candidates) {
    if (await isFile(candidate)) return candidate
  }

  throw new Error(`Unable to resolve ${specifier} from ${formatPath(fromDir)}`)
}

async function listSourceFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true })
  const files: string[] = []

  for (const entry of entries) {
    const path = join(directory, entry.name)
    const relativePath = formatPath(path)
    // Tests and the `/testing` entries are test code: they may depend upward.
    if (entry.isDirectory()) {
      if (entry.name === '__tests__' || entry.name === 'testing') continue
      files.push(...(await listSourceFiles(path)))
    } else if (entry.isFile() && entry.name.endsWith('.ts') && entry.name !== 'testing.ts') {
      files.push(path)
    }
  }

  return files
}

async function listProductionSourceFiles(): Promise<string[]> {
  const files = await Promise.all([...productionPackageDirectories.values()].map((directory) =>
    listSourceFiles(resolveRepoPath(`${directory}/src`)),
  ))
  return files.flat()
}

// The workspaces are the packages (packages/tinyjs and packages/fc-helper are Rust crates).
async function readWorkspacePackages(root: PackageManifest): Promise<Map<string, WorkspacePackage>> {
  const packages = new Map<string, WorkspacePackage>()
  for (const directory of root.workspaces ?? []) {
    const manifest = await readPackageManifest(resolveRepoPath(`${directory}/package.json`))
    const developmentExports = new Map<string, string>()
    for (const [subpath, target] of Object.entries(manifest.exports ?? {})) {
      if (subpath.includes('*')) continue
      const file = typeof target === 'string' ? target : target.development
      if (file) developmentExports.set(subpath, file)
    }
    packages.set(manifest.name, { directory, manifest, developmentExports })
  }
  return packages
}

// The ```text block under "## Production Dependency Graph": one `name -> dep, dep` line per package, `none` for a leaf.
async function readDocumentedDependencyGraph(): Promise<Map<string, readonly string[]>> {
  const doc = await readFile(resolveRepoPath('docs/package-boundaries.md'), 'utf8')
  const section = doc.split(/^## Production Dependency Graph$/m)[1]
  const block = section ? /```text\n([\s\S]*?)```/.exec(section)?.[1] : undefined
  if (!block) throw new Error('docs/package-boundaries.md has no ```text graph under "## Production Dependency Graph"')

  const graph = new Map<string, readonly string[]>()
  for (const line of block.trim().split('\n')) {
    const match = /^([\w-]+) -> (.+)$/.exec(line.trim())
    if (!match) throw new Error(`Unreadable dependency graph line: ${line}`)
    const [, name, dependencies] = match
    graph.set(`@demicodes/${name}`, dependencies === 'none' ? [] : dependencies!.split(',').map((dependency) => `@demicodes/${dependency.trim()}`).sort())
  }
  return graph
}

async function hasTestFiles(directory: string): Promise<boolean> {
  if (!(await isDirectory(directory))) return false
  const entries = await readdir(directory, { recursive: true })
  return entries.some((entry) => entry.endsWith('.test.ts'))
}

function sortedEntries(record: Record<string, string[]>): [string, string[]][] {
  return Object.entries(record).sort(([a], [b]) => a.localeCompare(b))
}

async function readPackageManifest(path: string): Promise<PackageManifest> {
  return JSON.parse(await readFile(path, 'utf8')) as PackageManifest
}

function productionDependencyNames(manifest: PackageManifest | undefined): string[] {
  return Object.keys(manifest?.dependencies ?? {}).sort()
}

function packageDependencyNames(manifest: PackageManifest | undefined): string[] {
  if (!manifest) return []
  return [
    ...Object.keys(manifest.dependencies ?? {}),
    ...Object.keys(manifest.devDependencies ?? {}),
    ...Object.keys(manifest.peerDependencies ?? {}),
    ...Object.keys(manifest.optionalDependencies ?? {}),
  ].sort()
}

interface WorkspacePackage {
  directory: string
  manifest: PackageManifest
  developmentExports: Map<string, string>
}

interface PackageManifest {
  name: string
  workspaces?: string[]
  scripts?: Record<string, string>
  exports?: Record<string, string | { development?: string }>
  dependencies?: Record<string, string>
  devDependencies?: Record<string, string>
  peerDependencies?: Record<string, string>
  optionalDependencies?: Record<string, string>
}

interface ProductionImportEdge {
  fromPackage: string
  toPackage: string
  file: string
  specifier: string
}

async function collectProductionWorkspaceImportEdges(): Promise<ProductionImportEdge[]> {
  const edges: ProductionImportEdge[] = []

  for (const [packageName, packageDirectory] of productionPackageDirectories) {
    const sourceDirectory = resolveRepoPath(`${packageDirectory}/src`)
    if (!(await isDirectory(sourceDirectory))) continue

    const files = await listSourceFiles(sourceDirectory)
    for (const file of files) {
      const source = await readFile(file, 'utf8')
      for (const specifier of findModuleSpecifiers(source)) {
        const toPackage = await resolveWorkspacePackageDependency(file, specifier)
        if (!toPackage || toPackage === packageName) continue
        edges.push({
          fromPackage: packageName,
          toPackage,
          file: formatPath(file),
          specifier,
        })
      }
    }
  }

  return edges
}

async function resolveWorkspacePackageDependency(fromFile: string, specifier: string): Promise<string | null> {
  const directPackage = workspacePackageNameFromSpecifier(specifier)
  if (directPackage) return directPackage
  if (!specifier.startsWith('.')) return null

  const resolved = await resolveLocalModule(dirname(fromFile), specifier)
  return workspacePackageNameForFile(resolved)
}

function workspacePackageNameFromSpecifier(specifier: string): string | null {
  const match = /^@demicodes\/[^/]+/.exec(specifier)
  if (!match) return null

  const packageName = match[0]
  return productionPackageDirectories.has(packageName) ? packageName : null
}

function workspacePackageNameForFile(path: string): string | null {
  const relativePath = formatPath(path)
  for (const [packageName, packageDirectory] of productionPackageDirectories) {
    if (relativePath === packageDirectory || relativePath.startsWith(`${packageDirectory}/`)) return packageName
  }
  return null
}

function findPackageDependencyCycle(edges: ProductionImportEdge[]): string | null {
  const adjacency = new Map<string, Set<string>>()
  for (const packageName of productionPackageDirectories.keys()) adjacency.set(packageName, new Set())
  for (const edge of edges) adjacency.get(edge.fromPackage)?.add(edge.toPackage)

  const visited = new Set<string>()
  const visiting = new Set<string>()
  const stack: string[] = []

  const visit = (packageName: string): string[] | null => {
    visited.add(packageName)
    visiting.add(packageName)
    stack.push(packageName)

    for (const dependency of adjacency.get(packageName) ?? []) {
      if (!adjacency.has(dependency)) continue
      if (visiting.has(dependency)) return [...stack.slice(stack.indexOf(dependency)), dependency]
      if (!visited.has(dependency)) {
        const cycle = visit(dependency)
        if (cycle) return cycle
      }
    }

    stack.pop()
    visiting.delete(packageName)
    return null
  }

  for (const packageName of adjacency.keys()) {
    if (visited.has(packageName)) continue
    const cycle = visit(packageName)
    if (cycle) return cycle.join(' -> ')
  }

  return null
}

function hasRuntimeImportFromAgent(source: string, name: string): boolean {
  const pattern = /\bimport\s+([\s\S]*?)\s+from\s+['"]@demicodes\/agent['"]/g
  let match = pattern.exec(source)
  while (match) {
    if (importClauseHasRuntimeName(match[1] ?? '', name)) return true
    match = pattern.exec(source)
  }
  return false
}

function hasRuntimeImportFromPackage(source: string, specifier: string): boolean {
  const escapedSpecifier = specifier.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const staticPattern = new RegExp(`\\b(?:import|export)\\s+([\\s\\S]*?)\\s+from\\s+['"]${escapedSpecifier}['"]`, 'g')
  let staticMatch = staticPattern.exec(source)
  while (staticMatch) {
    if (importClauseHasRuntimeBinding(staticMatch[1] ?? '')) return true
    staticMatch = staticPattern.exec(source)
  }

  const dynamicPattern = new RegExp(`\\bimport\\(\\s*['"]${escapedSpecifier}['"]\\s*\\)`)
  return dynamicPattern.test(source)
}

function importClauseHasRuntimeName(clause: string, name: string): boolean {
  const trimmed = clause.trim()
  if (trimmed.startsWith('type ')) return false

  const namedStart = trimmed.indexOf('{')
  const namedEnd = trimmed.lastIndexOf('}')
  if (namedStart === -1 || namedEnd === -1) return trimmed === name || trimmed.startsWith(`${name},`)

  const namedImports = trimmed
    .slice(namedStart + 1, namedEnd)
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)

  return namedImports.some((part) => {
    if (part.startsWith('type ')) return false
    const importedName = part.split(/\s+as\s+|\s+/)[0]
    return importedName === name
  })
}

function importClauseHasRuntimeBinding(clause: string): boolean {
  const trimmed = clause.trim()
  if (trimmed.startsWith('type ')) return false
  if (trimmed === '*') return true

  const namedStart = trimmed.indexOf('{')
  const namedEnd = trimmed.lastIndexOf('}')
  if (namedStart === -1 || namedEnd === -1) return true
  if (trimmed.slice(0, namedStart).trim().length > 0) return true

  const namedImports = trimmed
    .slice(namedStart + 1, namedEnd)
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)

  return namedImports.some((part) => !part.startsWith('type '))
}

async function isFile(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isFile()
  } catch {
    return false
  }
}

async function isDirectory(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isDirectory()
  } catch {
    return false
  }
}

function resolveRepoPath(path: string): string {
  return resolve(repoRoot, path)
}

function formatPath(path: string): string {
  return path.replace(repoRoot, '').replace(/^[/\\]/, '').split(sep).join('/')
}
