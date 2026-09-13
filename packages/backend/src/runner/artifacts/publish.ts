import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { canonicalJson, contentDigest, nativePackageSchema, NATIVE_TARGETS, type ArtifactResolver, type NativePackage } from '@demicodes/command-protocol'
import type { ArtifactStore, ObjectSource } from './store'

export interface PackageRelease {
  directory: string
  executable: string
}

interface VerifiedRelease {
  descriptor: NativePackage
  artifacts: ObjectSource[]
}

export interface PublishedPackages {
  packages: NativePackage[]
  resolveArtifact: ArtifactResolver
}

async function verify(release: PackageRelease, signal: AbortSignal): Promise<VerifiedRelease> {
  if (!/^[A-Za-z0-9_-]+$/.test(release.executable))
    throw new Error('Native release executable must be a basename without an extension')
  const directory = resolve(release.directory)
  const descriptor = nativePackageSchema.parse(JSON.parse(await readFile(join(directory, 'descriptor.json'), 'utf8')))
  const artifacts: ObjectSource[] = []
  for (const target of NATIVE_TARGETS) {
    signal.throwIfAborted()
    const path = join(directory, target, `${release.executable}${target.includes('windows') ? '.exe' : ''}`)
    const expected = descriptor.targets[target]
    const hash = createHash('sha256')
    const md5 = createHash('md5')
    let size = 0
    for await (const bytes of createReadStream(path, { signal })) {
      size += bytes.length
      if (size > expected.size)
        throw new Error(`Native artifact exceeds declared size: ${target}`)
      hash.update(bytes)
      md5.update(bytes)
    }
    if (size !== expected.size || hash.digest('hex') !== expected.sha256)
      throw new Error(`Native artifact does not match descriptor: ${target}`)
    artifacts.push({ ...expected, body: path, md5: md5.digest('base64') })
  }
  return { descriptor, artifacts }
}

/** Verify every release before publishing blobs; publish descriptors only after their blobs. */
export async function publishPackages(releases: readonly PackageRelease[], store: ArtifactStore,
  prefix: string, signal: AbortSignal): Promise<PublishedPackages> {
  if (!/^[A-Za-z0-9][A-Za-z0-9_/-]*$/.test(prefix) || prefix.endsWith('/'))
    throw new Error('Invalid native artifact object prefix')
  if (releases.length === 0)
    throw new Error('Native package releases are required')
  const verified: VerifiedRelease[] = []
  const ids = new Set<string>()
  for (const release of releases) {
    const result = await verify(release, signal)
    if (ids.has(result.descriptor.id))
      throw new Error(`Duplicate native package: ${result.descriptor.id}`)
    ids.add(result.descriptor.id)
    verified.push(result)
  }
  const artifacts = new Map<string, number>()
  const keyFor = (sha256: string) => `${prefix}/blobs/${sha256}`
  for (const release of verified) {
    for (const artifact of release.artifacts) {
      signal.throwIfAborted()
      if (!artifacts.has(artifact.sha256)) {
        await store.putImmutable(keyFor(artifact.sha256), artifact, signal)
        artifacts.set(artifact.sha256, artifact.size)
      }
    }
    const descriptor = release.descriptor
    const sha256 = await contentDigest(descriptor)
    const body = Buffer.from(canonicalJson(descriptor))
    const source = { body, size: body.length, sha256, md5: createHash('md5').update(body).digest('base64') }
    await store.putImmutable(`${prefix}/descriptors/${sha256}.json`, source, signal)
    // This is the immutable package/version claim, published last. A conflicting
    // descriptor fails instead of changing which bytes that version names.
    await store.putImmutable(`${prefix}/packages/${encodeURIComponent(descriptor.id)}/${encodeURIComponent(descriptor.version)}.json`, source, signal)
  }
  return {
    packages: verified.map(release => release.descriptor),
    resolveArtifact: async (artifact, signal) => {
      signal.throwIfAborted()
      if (artifacts.get(artifact.sha256) !== artifact.size)
        throw new Error('Artifact is not in the published package catalog')
      const expiresIn = 300
      const expiresAt = Date.now() + expiresIn * 1000
      const url = await store.signGet(keyFor(artifact.sha256), expiresIn, signal)
      if (new URL(url).protocol !== 'https:')
        throw new Error('Native artifact download requires HTTPS')
      return { url, expiresAt }
    },
  }
}
