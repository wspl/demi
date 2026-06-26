import { expect, test } from 'bun:test'
import { readFile, readdir, stat } from 'node:fs/promises'
import { dirname, join, resolve, sep } from 'node:path'
import { parseSync } from 'oxc-parser'

const repoRoot = resolve(import.meta.dir, '../../../..')

const platformNeutralEntries = [
  ['@demi/utils', 'packages/utils/src/index.ts'],
  ['@demi/core', 'packages/core/src/index.ts'],
  ['@demi/provider', 'packages/provider/src/index.ts'],
  ['@demi/agent', 'packages/agent/src/index.ts'],
  ['@demi/shell', 'packages/shell/src/index.ts'],
  ['@demi/coding-agent', 'packages/coding-agent/src/index.ts'],
] as const

const workspaceEntries = new Map<string, string>([
  ...platformNeutralEntries,
  ['@demi/provider-claude-code', 'packages/provider-claude-code/src/index.ts'],
  ['@demi/provider-codex', 'packages/provider-codex/src/index.ts'],
  ['@demi/provider-openai-api', 'packages/provider-openai-api/src/index.ts'],
  ['@demi/provider-anthropic-api', 'packages/provider-anthropic-api/src/index.ts'],
  ['@demi/host-local', 'packages/host-local/src/index.ts'],
  ['@demi/repl', 'packages/repl/src/index.ts'],
])

const productionPackageDirectories = new Map<string, string>([
  ['@demi/utils', 'packages/utils'],
  ['@demi/core', 'packages/core'],
  ['@demi/provider', 'packages/provider'],
  ['@demi/shell', 'packages/shell'],
  ['@demi/host-local', 'packages/host-local'],
  ['@demi/agent', 'packages/agent'],
  ['@demi/coding-agent', 'packages/coding-agent'],
  ['@demi/provider-claude-code', 'packages/provider-claude-code'],
  ['@demi/provider-codex', 'packages/provider-codex'],
  ['@demi/provider-openai-api', 'packages/provider-openai-api'],
  ['@demi/provider-anthropic-api', 'packages/provider-anthropic-api'],
  ['@demi/repl', 'packages/repl'],
])

const productionDependencyGraph = new Map<string, readonly string[]>([
  ['@demi/utils', []],
  ['@demi/core', []],
  ['@demi/provider', ['@demi/core']],
  ['@demi/shell', ['@demi/utils']],
  ['@demi/host-local', ['@demi/shell']],
  ['@demi/agent', ['@demi/core', '@demi/provider', '@demi/shell', '@demi/utils']],
  ['@demi/coding-agent', ['@demi/agent', '@demi/core', '@demi/shell', '@demi/utils']],
  ['@demi/provider-claude-code', ['@demi/core', '@demi/provider', '@demi/utils']],
  ['@demi/provider-codex', ['@demi/core', '@demi/provider', '@demi/utils']],
  ['@demi/provider-openai-api', ['@demi/core', '@demi/provider', '@demi/utils']],
  ['@demi/provider-anthropic-api', ['@demi/core', '@demi/provider', '@demi/utils']],
  [
    '@demi/repl',
    [
      '@demi/agent',
      '@demi/coding-agent',
      '@demi/core',
      '@demi/host-local',
      '@demi/provider',
      '@demi/provider-anthropic-api',
      '@demi/provider-claude-code',
      '@demi/provider-codex',
      '@demi/provider-openai-api',
      '@demi/shell',
      '@demi/utils',
    ],
  ],
])

const allowedWorkspaceSubpaths = new Map<string, string>([
  ['just-bash/ast/types', 'packages/just-bash/packages/just-bash/src/ast/types.ts'],
  ['just-bash/interpreter/helpers/ifs', 'packages/just-bash/packages/just-bash/src/interpreter/helpers/ifs.ts'],
  ['just-bash/parser', 'packages/just-bash/packages/just-bash/src/parser/parser.ts'],
  ['@demi/shell/storage', 'packages/shell/src/storage.ts'],
])

const nodeOnlySubpaths = new Map<string, string>([
  ['@demi/agent/stdio', 'packages/agent/src/stdio-transport.ts'],
])

const forbiddenSourcePatterns = [
  ['node builtin import', /\b(?:import|export)\s+(?:type\s+)?(?:[^'"]*?\s+from\s+)?['"]node:/],
  ['node builtin require', /\brequire\(\s*['"]node:/],
  ['Buffer global', /\bBuffer\b/],
  ['process env/cwd', /\bprocess\.(?:env|cwd)\b/],
] as const

const neutralPackageLeakPatterns = [
  ['concrete provider package reference', /@demi\/provider-(?:claude-code|codex|openai-api|anthropic-api)\b|provider-(?:claude-code|codex|openai-api|anthropic-api)/i],
  ['concrete provider implementation class', /\b(?:ClaudeCodeProvider|CodexProvider|OpenAIApiProvider|AnthropicApiProvider)\b/],
  ['concrete catalog source label', /\b(?:codex-backend|models\.dev)\b/i],
  ['provider backend identifier', /\b(?:backend-api|chatgpt\.com|api\.openai\.com|responses_websockets)\b/i],
  ['concrete provider product name', /\bClaude Code\b|\bOpenAI Codex\b/i],
] as const

const staticSpecifierPattern = /\b(?:import|export)\s+(?:type\s+)?(?:[^'"]*?\s+from\s+)?['"]([^'"]+)['"]/g
const dynamicSpecifierPattern = /\bimport\(\s*['"]([^'"]+)['"]\s*\)/g

const nodeOnlyFiles = new Set([...nodeOnlySubpaths.values()].map(resolveRepoPath))

for (const [entryName, entryPath] of platformNeutralEntries) {
  test(`${entryName} root entry has no Node-only source in its static closure`, async () => {
    const violations = await findPlatformViolations(resolveRepoPath(entryPath))

    expect(violations).toEqual([])
  })
}

test('only AgentServer imports AgentSession as a runtime value outside tests', async () => {
  const files = await listSourceFiles(resolveRepoPath('packages'))
  const violations: string[] = []

  for (const file of files) {
    const relativeFile = formatPath(file)
    const source = await readFile(file, 'utf8')
    if (!hasRuntimeImportFromAgent(source, 'AgentSession')) continue
    if (relativeFile !== 'packages/agent/src/server.ts') violations.push(relativeFile)
  }

  expect(violations).toEqual([])
})

test('runtime source uses the forked bash package without embedded upstream snapshots', async () => {
  const forbiddenDirs = [
    'packages/bash',
    'packages/shell/vendor',
    'packages/shell/src/internal/just-bash',
    'packages/just-bash/upstream',
    'packages/just-bash/src',
  ]
  const existingForbiddenDirs: string[] = []
  for (const directory of forbiddenDirs) {
    if (await isDirectory(resolveRepoPath(directory))) existingForbiddenDirs.push(directory)
  }

  const files = await listSourceFiles(resolveRepoPath('packages'))
  const violations: string[] = []

  for (const file of files) {
    const source = await readFile(file, 'utf8')
    for (const specifier of findModuleSpecifiers(source)) {
      if (specifier.includes('vendor/just-bash') || specifier.includes('just-bash/upstream') || specifier.includes('@demi/just-bash')) violations.push(`${formatPath(file)} imports ${specifier}`)
    }
  }

  expect([...existingForbiddenDirs, ...violations]).toEqual([])
})

test('@demi/shell does not depend on the agent runtime', async () => {
  const files = await listSourceFiles(resolveRepoPath('packages/shell/src'))
  const violations: string[] = []

  for (const file of files) {
    const source = await readFile(file, 'utf8')
    if (hasRuntimeImportFromPackage(source, '@demi/agent')) violations.push(formatPath(file))
  }

  expect(violations).toEqual([])
})

test('package manifests preserve layering boundaries', async () => {
  const manifests = await readPackageManifests()

  expect(packageDependencyNames(manifests.get('@demi/shell')).filter((name) => name === '@demi/core' || name === '@demi/provider')).toEqual([])

  const platformNeutralPackages = [
    '@demi/utils',
    '@demi/core',
    '@demi/provider',
    '@demi/agent',
    '@demi/shell',
    '@demi/coding-agent',
  ]
  for (const packageName of platformNeutralPackages) {
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demi/host-local')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demi/provider-claude-code')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demi/provider-codex')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demi/provider-openai-api')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demi/provider-anthropic-api')
  }

  const claudeProviderDependencies = packageDependencyNames(manifests.get('@demi/provider-claude-code'))
  expect(claudeProviderDependencies.some((name) => name === '@anthropic-ai/claude-agent-sdk' || name.includes('claude-agent-sdk'))).toBe(false)

  const webUiDependencies = packageDependencyNames(manifests.get('@demi/web-ui'))
  for (const forbidden of [
    '@demi/host-local',
    '@demi/shell',
    '@demi/coding-agent',
    '@demi/provider-claude-code',
    '@demi/provider-codex',
    '@demi/provider-openai-api',
    '@demi/provider-anthropic-api',
    '@demi/repl',
    '@demi/web',
  ]) {
    expect(webUiDependencies).not.toContain(forbidden)
  }
})

test('@demi/core and @demi/provider contain no concrete provider product details', async () => {
  const scopes = [
    ['@demi/core', 'packages/core/src'],
    ['@demi/provider', 'packages/provider/src'],
  ] as const
  const violations: string[] = []

  for (const [packageName, directory] of scopes) {
    const files = await listSourceFiles(resolveRepoPath(directory))
    for (const file of files) {
      const source = await readFile(file, 'utf8')
      const semanticSource = sourceSemanticText(file, source)
      for (const [label, pattern] of neutralPackageLeakPatterns) {
        if (pattern.test(semanticSource)) violations.push(`${packageName}: ${formatPath(file)} contains ${label}`)
      }
    }
  }

  expect(violations.sort()).toEqual([])
})

test('production source dependency graph follows documented package boundaries', async () => {
  const edges = await collectProductionWorkspaceImportEdges()
  const violations: string[] = []

  for (const edge of edges) {
    const allowed = productionDependencyGraph.get(edge.fromPackage) ?? []
    if (!allowed.includes(edge.toPackage)) violations.push(`${edge.file} imports ${edge.specifier} (${edge.fromPackage} -> ${edge.toPackage})`)
  }

  expect([...new Set(violations)].sort()).toEqual([])
  expect(findPackageDependencyCycle(edges)).toBeNull()
})

test('production workspace imports are declared as package dependencies', async () => {
  const manifests = await readPackageManifests()
  const edges = await collectProductionWorkspaceImportEdges()
  const violations: string[] = []

  for (const edge of edges) {
    const manifest = manifests.get(edge.fromPackage)
    if (manifest?.dependencies?.[edge.toPackage]) continue
    violations.push(`${edge.fromPackage} imports ${edge.toPackage} in ${edge.file} but does not declare it in dependencies`)
  }

  expect([...new Set(violations)].sort()).toEqual([])
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
    ...utilsHelperNames.map((name) => ({ name, home: 'packages/utils/', pkg: '@demi/utils' })),
    // zeroUsage returns core's TokenUsage, so its canonical home is @demi/core (utils cannot depend on core).
    { name: 'zeroUsage', home: 'packages/core/', pkg: '@demi/core' },
  ]
  const files = await listSourceFiles(resolveRepoPath('packages'))
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

function sourceSemanticText(file: string, source: string): string {
  const parsed = parseSync(formatPath(file), source, { sourceType: 'module' })
  if (parsed.errors.length > 0) {
    const messages = parsed.errors.map((error) => error.message).join('; ')
    throw new Error(`Unable to parse ${formatPath(file)} for boundary checks: ${messages}`)
  }

  const strings: string[] = []
  collectAstStrings(parsed.program, strings)
  return strings.join('\n')
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

  const workspaceEntry = workspaceEntries.get(specifier) ?? allowedWorkspaceSubpaths.get(specifier)
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
    if (entry.isDirectory()) {
      if (relativePath === 'packages/just-bash') continue
      if (entry.name === '__tests__') continue
      files.push(...(await listSourceFiles(path)))
    } else if (entry.isFile() && entry.name.endsWith('.ts')) {
      files.push(path)
    }
  }

  return files
}

async function readPackageManifests(): Promise<Map<string, PackageManifest>> {
  const manifests = new Map<string, PackageManifest>()
  const rootManifest = await readPackageManifest(resolveRepoPath('package.json'))
  manifests.set(rootManifest.name, rootManifest)

  const packageDirs = await readdir(resolveRepoPath('packages'), { withFileTypes: true })
  for (const entry of packageDirs) {
    if (!entry.isDirectory()) continue
    const manifest = await readPackageManifest(resolveRepoPath(`packages/${entry.name}/package.json`))
    manifests.set(manifest.name, manifest)
  }

  return manifests
}

async function readPackageManifest(path: string): Promise<PackageManifest> {
  return JSON.parse(await readFile(path, 'utf8')) as PackageManifest
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

interface PackageManifest {
  name: string
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
  const match = /^@demi\/[^/]+/.exec(specifier)
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
  const pattern = /\bimport\s+([\s\S]*?)\s+from\s+['"]@demi\/agent['"]/g
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
