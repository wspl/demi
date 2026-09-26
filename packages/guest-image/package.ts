import { createHash } from 'node:crypto'
import { chmod, copyFile, mkdir, readFile, readdir, rename, rm } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { z } from 'zod'
import { nativePackageSchema, type NativeArtifact } from '@demicodes/command-protocol'
import { browserReleaseSchema } from '@demicodes/browser-protocol'
import { runnerReleaseSchema } from '@demicodes/runner-protocol/release'
import { cloudImageManifestSchema } from '../machines/src/image-manifest'
import { fileArtifact } from '../../scripts/native/release-files'
import { requireTool } from '../machines/src/gvisor/image-tools'
import { atomicJson, syncFile } from '../machines/src/machine-image-store'

const { values } = parseArgs({ options: {
  root: { type: 'string' }, arch: { type: 'string' }, output: { type: 'string' },
  package: { type: 'string', multiple: true },
  'runner-release': { type: 'string' },
  'uv-version': { type: 'string' }, 'uv-archive': { type: 'string' },
} })
const options = z.object({ root: z.string(), arch: z.enum(['aarch64', 'x86_64']), output: z.string(), package: z.array(z.string()).min(1), 'runner-release': z.string(), 'uv-version': z.string(), 'uv-archive': z.string() }).parse(values)
const root = resolve(options.root)
const output = resolve(options.output)
const target = `${options.arch}-unknown-linux-musl` as const
const executables: Record<string, NativeArtifact> = {}
const releases = []
const runner = runnerReleaseSchema.parse(JSON.parse(await readFile(join(options['runner-release'], 'manifest.json'), 'utf8')))
for (const directory of options.package) {
  const release = nativePackageSchema.parse(JSON.parse(await readFile(join(directory, 'descriptor.json'), 'utf8')))
  const artifact = release.targets[target]
  if (!artifact) throw new Error(`Release ${release.id} lacks ${target}`)
  const candidates = await readdir(join(directory, target))
  if (candidates.length !== 1) throw new Error(`Release ${release.id} must contain one target executable`)
  const executable = candidates[0]!
  const source = join(directory, target, executable)
  const actual = await fileArtifact(source)
  if (actual.sha256 !== artifact.sha256 || actual.size !== artifact.size) throw new Error('Native artifact integrity mismatch')
  const path = `/opt/demi/artifacts/${artifact.sha256}/${executable}`
  await mkdir(join(root, 'opt/demi/artifacts', artifact.sha256), { recursive: true })
  await copyFile(source, join(root, path))
  await chmod(join(root, path), 0o755)
  executables[path] = artifact
  releases.push(release)
}
for (const path of ['/usr/bin/demi-runner', '/usr/bin/tini', '/usr/local/bin/uv', '/usr/local/bin/uvx']) {
  executables[path] = await fileArtifact(join(root, path))
}
const inventory = await requireTool('chroot', [root, 'dpkg-query', '-W', '-f=${Package}\t${Version}\n'])
const packages = inventory.split('\n').map(line => {
  const [name, version] = z.tuple([z.string().min(1), z.string().min(1)]).parse(line.split('\t'))
  return { name, version }
})
const browser = browserReleaseSchema.parse(JSON.parse(await readFile(new URL('../../crates/builtin-protocol/src/release/chrome.json', import.meta.url), 'utf8')))
const browserKey = options.arch === 'aarch64' ? 'aarch64-unknown-linux-musl' : 'x86_64-unknown-linux-musl'
const browserArchive = browser.platforms.find(platform => platform.target === browserKey)
if (!browserArchive) throw new Error('Missing Cloud browser target')
const chrome = `/opt/demi/browsers/${browserArchive.sha256}/${browserArchive.executable}`
executables[chrome] = await fileArtifact(join(root, chrome))
const stage = `${output}.stage-${crypto.randomUUID()}`
await mkdir(stage, { recursive: true })
try {
  const archive = join(stage, 'rootfs.tar.zst')
  await requireTool('tar', ['--numeric-owner', '--xattrs', '--acls', '--zstd', '-cf', archive, '-C', root, '.'], undefined, 300_000)
  const manifest = cloudImageManifestSchema.parse({
    formatVersion: 1, os: 'linux', architecture: options.arch === 'aarch64' ? 'arm64' : 'amd64',
    ubuntu: '26.04', packages, executables, releases, runner,
    rootfs: { file: 'rootfs.tar.zst', ...await fileArtifact(archive) },
    tools: [
      { name: 'uv', version: options['uv-version'], sha256: (await fileArtifact(options['uv-archive'])).sha256 },
      { name: 'chrome', version: browser.version, sha256: browserArchive.sha256 },
    ],
  })
  await atomicJson(join(stage, 'manifest.json'), manifest)
  await syncFile(archive)
  await syncFile(stage)
  // Releases are immutable. Choose a new output directory when rebuilding.
  await rename(stage, output)
  await syncFile(resolve(output, '..'))
  console.log(createHash('sha256').update(await readFile(join(output, 'manifest.json'))).digest('hex'))
} finally {
  await rm(stage, { recursive: true, force: true })
}
