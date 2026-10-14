import { existsSync, readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'

const publicRegistry = 'https://registry.npmjs.org/'
// bun.lock writes a package's tarball URL only when it came from a registry
// other than the default one, so any URL in it names a package another
// registry (a mirror, a private registry) resolved.
const lockfile = 'bun.lock'

const failures: string[] = []

assertFileContains(
  '.npmrc',
  /^registry\s*=\s*https:\/\/registry\.npmjs\.org\/?\s*$/m
)
assertFileContains(
  'bunfig.toml',
  /registry\s*=\s*"https:\/\/registry\.npmjs\.org\/?"/
)
assertNpmConfig()
scanLockfile()

if (failures.length > 0) {
  for (const failure of failures) console.error(failure)
  process.exit(1)
}

console.log(`registry: ${publicRegistry}`)

function assertFileContains(file: string, pattern: RegExp): void {
  if (!existsSync(file)) {
    failures.push(`${file} is missing`)
    return
  }
  const content = readFileSync(file, 'utf8')
  if (!pattern.test(content))
    failures.push(`${file} must pin ${publicRegistry}`)
}

function assertNpmConfig(): void {
  const result = spawnSync(
    'npm',
    ['config', 'get', 'registry'],
    { encoding: 'utf8' }
  )
  if (result.status !== 0) {
    failures.push(`npm config get registry failed: ${result.stderr.trim()}`)
    return
  }
  if (normalizeRegistry(result.stdout.trim()) !== normalizeRegistry(publicRegistry)) {
    failures.push(
      `npm registry is ${result.stdout.trim()}, expected ${publicRegistry}`
    )
  }
}

function scanLockfile(): void {
  if (!existsSync(lockfile)) {
    failures.push(`${lockfile} is missing`)
    return
  }
  const urls = readFileSync(lockfile, 'utf8').match(/https?:\/\/[^\s"']+/g) ?? []
  for (const url of new Set(urls)) {
    if (!url.startsWith(publicRegistry))
      failures.push(`${lockfile} resolves a package from ${url}, not ${publicRegistry}`)
  }
}

function normalizeRegistry(value: string): string {
  return value.endsWith('/') ? value : `${value}/`
}
