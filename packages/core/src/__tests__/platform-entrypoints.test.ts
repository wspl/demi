import { expect, test } from 'bun:test'
import { readFile, readdir, stat } from 'node:fs/promises'
import { dirname, join, resolve, sep } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')

const platformNeutralEntries = [
  ['@demi/core', 'packages/core/src/index.ts'],
  ['@demi/provider', 'packages/provider/src/index.ts'],
  ['@demi/agent', 'packages/agent/src/index.ts'],
  ['@demi/shell', 'packages/shell/src/index.ts'],
  ['@demi/coding-agent', 'packages/coding-agent/src/index.ts'],
] as const

const workspaceEntries = new Map<string, string>(platformNeutralEntries)

const allowedWorkspaceSubpaths = new Map<string, string>([
  ['just-bash/ast/types', 'packages/just-bash/packages/just-bash/src/ast/types.ts'],
  ['just-bash/interpreter/helpers/ifs', 'packages/just-bash/packages/just-bash/src/interpreter/helpers/ifs.ts'],
  ['just-bash/parser', 'packages/just-bash/packages/just-bash/src/parser/parser.ts'],
  ['@demi/shell/storage', 'packages/shell/src/storage.ts'],
])

const nodeOnlySubpaths = new Map<string, string>([
  ['@demi/shell/local-host', 'packages/shell/src/local-host.ts'],
  ['@demi/shell/store', 'packages/shell/src/store.ts'],
  ['@demi/agent/stdio', 'packages/agent/src/stdio-transport.ts'],
])

const forbiddenSourcePatterns = [
  ['node builtin import', /\b(?:import|export)\s+(?:type\s+)?(?:[^'"]*?\s+from\s+)?['"]node:/],
  ['node builtin require', /\brequire\(\s*['"]node:/],
  ['Buffer global', /\bBuffer\b/],
  ['process env/cwd', /\bprocess\.(?:env|cwd)\b/],
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
    '@demi/core',
    '@demi/provider',
    '@demi/agent',
    '@demi/shell',
    '@demi/coding-agent',
  ]
  for (const packageName of platformNeutralPackages) {
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demi/provider-claude-code')
    expect(packageDependencyNames(manifests.get(packageName))).not.toContain('@demi/provider-codex')
  }

  const claudeProviderDependencies = packageDependencyNames(manifests.get('@demi/provider-claude-code'))
  expect(claudeProviderDependencies.some((name) => name === '@anthropic-ai/claude-agent-sdk' || name.includes('claude-agent-sdk'))).toBe(false)
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
