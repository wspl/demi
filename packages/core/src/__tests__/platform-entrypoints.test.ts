import { expect, test } from 'bun:test'
import { readFile, readdir, stat } from 'node:fs/promises'
import { dirname, join, resolve, sep } from 'node:path'
import { parseSync, Visitor } from 'oxc-parser'

const repoRoot = resolve(import.meta.dir, '../../../..')

const platformNeutralEntries = [
  ['@demicodes/utils', 'packages/utils/src/index.ts'],
  ['@demicodes/core', 'packages/core/src/index.ts'],
  ['@demicodes/provider', 'packages/provider/src/index.ts'],
  ['@demicodes/agent', 'packages/agent/src/index.ts'],
  ['@demicodes/shell', 'packages/shell/src/index.ts'],
  ['@demicodes/coding-agent', 'packages/coding-agent/src/index.ts'],
  ['@demicodes/runner-protocol', 'packages/runner-protocol/src/index.ts'],
  ['@demicodes/host-virtual', 'packages/host-virtual/src/index.ts'],
  ['@demicodes/command-loader', 'packages/command-loader/src/index.ts'],
  ['@demicodes/tinybash', 'packages/tinybash/src/index.ts'],
  ['@demicodes/host-remote', 'packages/host-remote/src/index.ts'],
] as const

const workspaceEntries = new Map<string, string>([
  ...platformNeutralEntries,
  ['@demicodes/provider-claude-code', 'packages/provider-claude-code/src/index.ts'],
  ['@demicodes/provider-codex', 'packages/provider-codex/src/index.ts'],
  ['@demicodes/provider-openai-api', 'packages/provider-openai-api/src/index.ts'],
  ['@demicodes/provider-anthropic-api', 'packages/provider-anthropic-api/src/index.ts'],
  ['@demicodes/provider-grok-build', 'packages/provider-grok-build/src/index.ts'],
  ['@demicodes/provider-google', 'packages/provider-google/src/index.ts'],
  ['@demicodes/backend', 'packages/backend/src/index.ts'],
])

const productionPackageDirectories = new Map<string, string>([
  ['@demicodes/utils', 'packages/utils'],
  ['@demicodes/core', 'packages/core'],
  ['@demicodes/provider', 'packages/provider'],
  ['@demicodes/shell', 'packages/shell'],
  ['@demicodes/agent', 'packages/agent'],
  ['@demicodes/coding-agent', 'packages/coding-agent'],
  ['@demicodes/provider-claude-code', 'packages/provider-claude-code'],
  ['@demicodes/provider-codex', 'packages/provider-codex'],
  ['@demicodes/provider-openai-api', 'packages/provider-openai-api'],
  ['@demicodes/provider-anthropic-api', 'packages/provider-anthropic-api'],
  ['@demicodes/provider-grok-build', 'packages/provider-grok-build'],
  ['@demicodes/provider-google', 'packages/provider-google'],
  ['@demicodes/runner-protocol', 'packages/runner-protocol'],
  ['@demicodes/runner', 'packages/runner'],
  ['@demicodes/host-virtual', 'packages/host-virtual'],
  ['@demicodes/command-loader', 'packages/command-loader'],
  ['@demicodes/tinybash', 'packages/tinybash'],
  ['@demicodes/host-remote', 'packages/host-remote'],
  ['@demicodes/backend', 'packages/backend'],
])

const productionDependencyGraph = new Map<string, readonly string[]>([
  ['@demicodes/utils', []],
  ['@demicodes/core', []],
  ['@demicodes/provider', ['@demicodes/core', '@demicodes/utils']],
  ['@demicodes/shell', ['@demicodes/tinybash', '@demicodes/utils']],
  ['@demicodes/agent', ['@demicodes/core', '@demicodes/provider', '@demicodes/shell', '@demicodes/utils']],
  ['@demicodes/coding-agent', ['@demicodes/agent', '@demicodes/core', '@demicodes/shell', '@demicodes/utils']],
  ['@demicodes/provider-claude-code', ['@demicodes/core', '@demicodes/provider', '@demicodes/utils']],
  ['@demicodes/provider-codex', ['@demicodes/core', '@demicodes/provider', '@demicodes/utils']],
  ['@demicodes/provider-openai-api', ['@demicodes/core', '@demicodes/provider', '@demicodes/utils']],
  ['@demicodes/provider-anthropic-api', ['@demicodes/core', '@demicodes/provider', '@demicodes/utils']],
  ['@demicodes/provider-grok-build', ['@demicodes/core', '@demicodes/provider', '@demicodes/utils']],
  ['@demicodes/provider-google', ['@demicodes/core', '@demicodes/provider', '@demicodes/utils']],
  ['@demicodes/runner-protocol', ['@demicodes/shell', '@demicodes/utils']],
  ['@demicodes/host-virtual', ['@demicodes/shell', '@demicodes/tinybash', '@demicodes/utils']],
  ['@demicodes/command-loader', ['@demicodes/shell', '@demicodes/utils']],
  ['@demicodes/tinybash', ['@demicodes/utils']],
  ['@demicodes/host-remote', ['@demicodes/runner-protocol', '@demicodes/shell', '@demicodes/utils']],
  [
    '@demicodes/backend',
    [
      '@demicodes/agent',
      '@demicodes/coding-agent',
      '@demicodes/command-loader',
      '@demicodes/core',
      '@demicodes/host-remote',
      '@demicodes/host-virtual',
      '@demicodes/provider',
      '@demicodes/provider-anthropic-api',
      '@demicodes/provider-claude-code',
      '@demicodes/provider-codex',
      '@demicodes/provider-google',
      '@demicodes/provider-grok-build',
      '@demicodes/provider-openai-api',
      '@demicodes/runner-protocol',
      '@demicodes/shell',
      '@demicodes/utils',
    ],
  ],
  [
    '@demicodes/runner',
    ['@demicodes/command-loader', '@demicodes/runner-protocol', '@demicodes/shell', '@demicodes/utils'],
  ],
])

const allowedWorkspaceSubpaths = new Map<string, string>([
  ['@demicodes/shell/storage', 'packages/shell/src/storage.ts'],
  ['@demicodes/shell/bash', 'packages/shell/src/bash.ts'],
  ['@demicodes/shell/host-fs', 'packages/shell/src/host-fs.ts'],
])

const nodeOnlySubpaths = new Map<string, string>([
  ['@demicodes/agent/stdio', 'packages/agent/src/protocol/stdio-transport.ts'],
])

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

const nodeOnlyFiles = new Set([...nodeOnlySubpaths.values()].map(resolveRepoPath))

for (const [entryName, entryPath] of platformNeutralEntries) {
  test(`${entryName} root entry has no Node-only source in its static closure`, async () => {
    const violations = await findPlatformViolations(resolveRepoPath(entryPath))

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
    if (relativeFile !== 'packages/agent/src/server/open-session.ts') violations.push(relativeFile)
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

  const files = await listProductionSourceFiles()
  const violations: string[] = []

  for (const file of files) {
    const source = await readFile(file, 'utf8')
    for (const specifier of findModuleSpecifiers(source)) {
    }
  }

  expect([...existingForbiddenDirs, ...violations]).toEqual([])
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

test('package manifests preserve layering boundaries', async () => {
  const manifests = await readPackageManifests()

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

async function readPackageManifests(): Promise<Map<string, PackageManifest>> {
  const manifests = new Map<string, PackageManifest>()
  const rootManifest = await readPackageManifest(resolveRepoPath('package.json'))
  manifests.set(rootManifest.name, rootManifest)

  // The workspaces are the packages; packages/tinyjs is a Rust crate.
  for (const workspace of rootManifest.workspaces ?? []) {
    const manifest = await readPackageManifest(resolveRepoPath(`${workspace}/package.json`))
    manifests.set(manifest.name, manifest)
  }

  return manifests
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

interface PackageManifest {
  name: string
  workspaces?: string[]
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
