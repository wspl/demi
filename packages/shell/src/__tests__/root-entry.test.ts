import { readFile, stat } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { expect, test } from 'bun:test'
import { HostBackedFileSystem, type Host } from '../index'

const shellRootEntry = resolve(import.meta.dir, '../index.ts')
const forbiddenRuntimePatterns = [
  ['node builtin import', /\b(?:import|export)\s+(?:type\s+)?(?:[^'"]*?\s+from\s+)?['"]node:/],
  ['node builtin require', /\brequire\(\s*['"]node:/],
  ['Buffer global', /\bBuffer\b/],
  ['process env/cwd', /\bprocess\.(?:env|cwd)\b/],
] as const
const moduleSpecifierPattern = /\b(?:import|export)\s+(?:type\s+)?(?:[^'"]*?\s+from\s+)?['"]([^'"]+)['"]/g

test('root entry exposes browser-safe Host contract and HostBackedFileSystem class', async () => {
  const host: Pick<Host, 'defaultCwd'> = { defaultCwd: '/' }
  expect(host.defaultCwd).toBe('/')

  const fs = new HostBackedFileSystem({
    defaultCwd: '/tmp',
    fs: {} as Host['fs'],
    process: { spawn: async () => { throw new Error('not used') } },
    store: {} as Host['store'],
  })
  expect(typeof fs.resolvePath).toBe('function')
  expect(fs.resolvePath('/a', 'b')).toBe('/a/b')
})

test('root entry local static closure does not import Node-only runtime source', async () => {
  expect(await findNodeRuntimeViolations(shellRootEntry)).toEqual([])
})

async function findNodeRuntimeViolations(entry: string): Promise<string[]> {
  const seen = new Set<string>()
  const pending = [entry]
  const violations: string[] = []

  while (pending.length > 0) {
    const file = pending.pop()
    if (!file || seen.has(file)) continue
    seen.add(file)

    const source = await readFile(file, 'utf8')
    for (const [label, pattern] of forbiddenRuntimePatterns) {
      if (pattern.test(source)) violations.push(`${formatPath(file)} contains ${label}`)
    }

    for (const specifier of findLocalModuleSpecifiers(source)) {
      pending.push(await resolveLocalModule(dirname(file), specifier))
    }
  }

  return violations.sort()
}

function findLocalModuleSpecifiers(source: string): string[] {
  const specifiers = new Set<string>()
  moduleSpecifierPattern.lastIndex = 0
  let match = moduleSpecifierPattern.exec(source)
  while (match) {
    const specifier = match[1]
    if (specifier?.startsWith('.')) specifiers.add(specifier)
    match = moduleSpecifierPattern.exec(source)
  }
  return [...specifiers]
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

async function isFile(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isFile()
  } catch {
    return false
  }
}

function formatPath(path: string): string {
  return path.startsWith(process.cwd()) ? path.slice(process.cwd().length + 1) : path
}
