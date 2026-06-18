import { existsSync, readFileSync, statSync } from 'node:fs'
import { spawnSync } from 'node:child_process'

const publicRegistry = 'https://registry.npmjs.org/'
const bannedHosts = [
  ['bnpm', 'byted', 'org'].join('.'),
  ['registry', 'npm', 'taobao', 'org'].join('.'),
  ['npmmirror', 'com'].join('.'),
]

const failures: string[] = []

assertFileContains('.npmrc', /^registry\s*=\s*https:\/\/registry\.npmjs\.org\/?\s*$/m)
assertFileContains('bunfig.toml', /registry\s*=\s*"https:\/\/registry\.npmjs\.org\/?"/)
assertNpmConfig()
scanRepository()

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
  if (!pattern.test(content)) failures.push(`${file} must pin ${publicRegistry}`)
}

function assertNpmConfig(): void {
  const result = spawnSync('npm', ['config', 'get', 'registry'], { encoding: 'utf8' })
  if (result.status !== 0) {
    failures.push(`npm config get registry failed: ${result.stderr.trim()}`)
    return
  }
  if (normalizeRegistry(result.stdout.trim()) !== normalizeRegistry(publicRegistry)) {
    failures.push(`npm registry is ${result.stdout.trim()}, expected ${publicRegistry}`)
  }
}

function scanRepository(): void {
  const result = spawnSync('git', ['ls-files', '--cached', '--others', '--exclude-standard'], { encoding: 'utf8' })
  if (result.status !== 0) {
    failures.push(`git ls-files failed: ${result.stderr.trim()}`)
    return
  }

  for (const file of result.stdout.split('\n')) {
    if (!file || file.includes('/node_modules/')) continue
    let stats: ReturnType<typeof statSync>
    try {
      stats = statSync(file)
    } catch {
      continue
    }
    if (!stats.isFile()) continue

    const content = readFileSync(file, 'utf8')
    for (const host of bannedHosts) {
      if (content.includes(host)) failures.push(`${file} contains forbidden registry host ${host}`)
    }
  }
}

function normalizeRegistry(value: string): string {
  return value.endsWith('/') ? value : `${value}/`
}
