import { createHash } from 'node:crypto'
import { mkdirSync, copyFileSync, readFileSync, writeFileSync, renameSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { commandClientBinary } from '../../command-client/build'
import { packRuntime } from './build'
import { bundleRuntime } from './bundle'
import { RUNNER_PROTOCOL_VERSION } from '../../runner-protocol/src/messages'
import local from '../../runner-protocol/src/local-contract.json'
import { runnerReleaseSchema } from '../../runner-protocol/src/release'

const targets = {
  'macos-arm64': 'arm64-macos',
  'macos-x64': 'x86_64-macos',
  'linux-arm64': 'aarch64-linux-musl',
  'linux-x64': 'x86_64-linux-musl',
} as const
const [directory, ...requested] = process.argv.slice(2)
if (!directory || !requested.length) throw new Error(`Usage: release.ts directory ${Object.keys(targets).join('|')} ...`)
const output = resolve(directory)
const stage = join(output, `.build-${process.pid}`)
mkdirSync(stage, { recursive: true })
const bundle = join(stage, 'entry.mjs')
await bundleRuntime(resolve(import.meta.dir, '../src/entry.ts'), bundle)
const entries: Record<string, { runner: string; client: string }> = {}
function fileHash(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}
for (const name of requested) {
  if (!(name in targets)) throw new Error(`unsupported release target: ${name}`)
  const target = targets[name as keyof typeof targets]
  const dir = join(stage, name)
  mkdirSync(dir, { recursive: true })
  packRuntime(bundle, join(dir, 'demi-runner'), target)
  copyFileSync(commandClientBinary(target), join(dir, 'demi'))
  entries[name] = {
    runner: fileHash(join(dir, 'demi-runner')),
    client: fileHash(join(dir, 'demi')),
  }
}
const contents = {
  wire: RUNNER_PROTOCOL_VERSION,
  local: local.version,
  targets: entries,
}
const release = createHash('sha256').update(JSON.stringify(contents)).digest('hex')
const releaseDir = join(output, release)
mkdirSync(releaseDir, { recursive: true })
for (const target of Object.keys(entries)) {
  mkdirSync(join(releaseDir, target), { recursive: true })
  for (const file of ['demi', 'demi-runner']) {
    copyFileSync(join(stage, target, file), join(releaseDir, target, file))
  }
}
const manifest = runnerReleaseSchema.parse({ release, ...contents })
const manifestJson = `${JSON.stringify(manifest, null, 2)}\n`
writeFileSync(join(releaseDir, 'manifest.json'), manifestJson)
writeFileSync(join(output, 'manifest.json.tmp'), manifestJson)
renameSync(join(output, 'manifest.json.tmp'), join(output, 'manifest.json'))
console.log(`Runner release: ${releaseDir}`)
