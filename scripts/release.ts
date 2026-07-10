/**
 * Publishes every public workspace package whose version is not yet on the
 * registry. Replaces `changeset publish`, which shells out to `npm publish`
 * and ships `workspace:^` literals verbatim (npm does not understand the
 * workspace protocol — the 0.3.0 release shipped broken tarballs this way).
 *
 * Pipeline per package: `bun pm pack` (rewrites workspace ranges from the
 * lockfile) → validate the packed manifest → `bun publish <tarball>`.
 *
 * Two failure modes are guarded against:
 *  - workspace protocol leaking into the tarball (the npm-publish bug);
 *  - bun rewriting against a stale lockfile (bun resolves workspace versions
 *    from bun.lock, and `bun install` does not refresh it after a version-only
 *    bump — so the lockfile is regenerated up front, and the validator then
 *    asserts every internal dep matches the live workspace version).
 *
 * Auth: a registry token from $NPM_TOKEN (Bun auto-loads .env) or an already
 * configured .npmrc. Usage: bun run release [--dry-run]
 */
import { $ } from 'bun'
import { readFileSync } from 'node:fs'
import { mkdtemp, readdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const ROOT = resolve(import.meta.dir, '..')
const REGISTRY = 'https://registry.npmjs.org'
const dryRun = process.argv.includes('--dry-run')

interface PackageManifest {
  name: string
  version: string
  private?: boolean
  dependencies?: Record<string, string>
  devDependencies?: Record<string, string>
  peerDependencies?: Record<string, string>
  optionalDependencies?: Record<string, string>
}

function readManifest(path: string): PackageManifest {
  return JSON.parse(readFileSync(path, 'utf8')) as PackageManifest
}

const rootManifest = JSON.parse(readFileSync(join(ROOT, 'package.json'), 'utf8')) as { workspaces: string[] }
const packages = rootManifest.workspaces
  .map((dir) => ({ dir: join(ROOT, dir), manifest: readManifest(join(ROOT, dir, 'package.json')) }))
  .filter(({ manifest }) => manifest.private !== true)

// Live workspace versions — the source of truth the packed tarballs must match.
const workspaceVersions = new Map(packages.map(({ manifest }) => [manifest.name, manifest.version]))

async function publishedVersions(name: string): Promise<Set<string>> {
  const response = await fetch(`${REGISTRY}/${name.replace('/', '%2F')}`)
  if (response.status === 404) return new Set()
  if (!response.ok) throw new Error(`Registry lookup for ${name} failed: ${response.status}`)
  const body = (await response.json()) as { versions?: Record<string, unknown> }
  return new Set(Object.keys(body.versions ?? {}))
}

function validatePackedManifest(packed: PackageManifest): string[] {
  const problems: string[] = []
  for (const section of ['dependencies', 'devDependencies', 'peerDependencies', 'optionalDependencies'] as const) {
    for (const [dep, range] of Object.entries(packed[section] ?? {})) {
      if (range.includes('workspace:')) {
        problems.push(`${section}.${dep} is "${range}" — workspace protocol leaked into the tarball`)
        continue
      }
      const live = workspaceVersions.get(dep)
      if (live !== undefined && range !== `^${live}`) {
        problems.push(`${section}.${dep} is "${range}", expected "^${live}" — stale lockfile rewrite`)
      }
    }
  }
  return problems
}

// Regenerate the lockfile so bun's workspace-range rewriting sees the live
// versions; a plain `bun install` after a version-only bump reports
// "no changes" and leaves stale versions in bun.lock.
console.log('Refreshing lockfile...')
await $`rm -f bun.lock`.cwd(ROOT)
await $`bun install`.cwd(ROOT).quiet()

console.log('Building...')
await $`bun run build`.cwd(ROOT).quiet()

const failures: string[] = []
let published = 0
for (const { dir, manifest } of packages) {
  const existing = await publishedVersions(manifest.name)
  if (existing.has(manifest.version)) {
    console.log(`skip    ${manifest.name}@${manifest.version} (already on registry)`)
    continue
  }

  const dest = await mkdtemp(join(tmpdir(), 'demi-release-'))
  await $`bun pm pack --destination ${dest}`.cwd(dir).quiet()
  const tarball = join(dest, (await readdir(dest)).find((f) => f.endsWith('.tgz'))!)
  const packed = JSON.parse(await $`tar -xOf ${tarball} package/package.json`.text()) as PackageManifest

  const problems = validatePackedManifest(packed)
  if (problems.length > 0) {
    failures.push(`${manifest.name}@${manifest.version}:\n  ${problems.join('\n  ')}`)
    console.error(`INVALID ${manifest.name}@${manifest.version}`)
    continue
  }

  if (dryRun) {
    console.log(`ok      ${manifest.name}@${manifest.version} (dry run, not published)`)
    continue
  }
  const env = { ...process.env, ...(process.env.NPM_TOKEN ? { NPM_CONFIG_TOKEN: process.env.NPM_TOKEN } : {}) }
  await $`bun publish ${tarball} --access public`.cwd(dir).env(env)
  console.log(`publish ${manifest.name}@${manifest.version}`)
  published += 1
}

if (failures.length > 0) {
  console.error(`\nAborted: ${failures.length} package(s) failed tarball validation; nothing further published.\n`)
  console.error(failures.join('\n'))
  process.exit(1)
}

if (!dryRun && published > 0) {
  await $`bun run changeset tag`.cwd(ROOT)
  console.log(`\nDone: ${published} package(s) published. Push tags with: git push origin --tags`)
} else {
  console.log(`\nDone: nothing published${dryRun ? ' (dry run)' : ''}.`)
}
