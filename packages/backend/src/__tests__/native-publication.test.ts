import { expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { canonicalJson, NATIVE_TARGETS, nativePackageSchema } from '@demicodes/command-protocol'
import { publishPackages, type PackageRelease } from '../runner/artifacts/publish'
import type { ArtifactStore, ObjectSource } from '../runner/artifacts/store'
import { S3ArtifactStore } from '../runner/artifacts/s3'

class MemoryStore implements ArtifactStore {
  readonly objects = new Map<string, Buffer>()
  readonly writes: string[] = []
  signatures = 0
  async putImmutable(key: string, source: ObjectSource, signal: AbortSignal): Promise<void> {
    signal.throwIfAborted()
    const body = Buffer.isBuffer(source.body) ? source.body : await readFile(source.body)
    expect(body.length).toBe(source.size)
    expect(createHash('sha256').update(body).digest('hex')).toBe(source.sha256)
    if (this.objects.has(key) && !this.objects.get(key)!.equals(body))
      throw new Error('immutable conflict')
    this.objects.set(key, body)
    this.writes.push(key)
  }
  async signGet(key: string): Promise<string> {
    return `https://store.example.test/${key}?signature=${++this.signatures}`
  }
  close(): void {}
}

async function release(directory: string, carried: readonly string[] = NATIVE_TARGETS): Promise<PackageRelease> {
  const targets: Record<string, { sha256: string; size: number }> = {}
  for (const target of carried) {
    const body = Buffer.from(`test-only artifact ${target}`)
    targets[target] = { sha256: createHash('sha256').update(body).digest('hex'), size: body.length }
    await mkdir(join(directory, target), { recursive: true })
    await writeFile(join(directory, target, `commands${target.includes('windows') ? '.exe' : ''}`), body)
  }
  await writeFile(join(directory, 'descriptor.json'), canonicalJson({ id: 'example.commands', version: '1', protocolVersion: 1,
    operations: ['fixture'], targets }))
  return { directory, executable: 'commands' }
}

test('all six blobs precede immutable descriptors and locations are freshly signed', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'demi-publish-'))
  try {
    const store = new MemoryStore()
    const published = await publishPackages([await release(directory)], store, 'native', new AbortController().signal)
    expect(store.writes).toHaveLength(8)
    expect(store.writes.slice(0, 6).every(key => key.startsWith('native/blobs/'))).toBe(true)
    expect(store.writes[6]).toStartWith('native/descriptors/')
    expect(store.writes[7]).toBe('native/packages/example.commands/1.json')
    const artifact = published.packages[0]!.targets[NATIVE_TARGETS[0]]!
    const signal = new AbortController().signal
    const first = await published.resolveArtifact(artifact, signal)
    const second = await published.resolveArtifact(artifact, signal)
    expect(first).not.toEqual(second)
    expect('expiresAt' in first && first.expiresAt).toBeGreaterThan(Date.now())
    await expect(published.resolveArtifact({ ...artifact, size: artifact.size + 1 }, signal)).rejects.toThrow('not in the published')
    const descriptor = nativePackageSchema.parse(JSON.parse(await readFile(join(directory, 'descriptor.json'), 'utf8')))
    descriptor.operations.push('changed')
    await writeFile(join(directory, 'descriptor.json'), canonicalJson(descriptor))
    await expect(publishPackages([{ directory, executable: 'commands' }], store, 'native', signal)).rejects.toThrow('immutable conflict')
    expect(JSON.parse(store.objects.get('native/packages/example.commands/1.json')!.toString()).operations).toEqual(['fixture'])
  } finally { await rm(directory, { recursive: true, force: true }) }
})

test('a missing or corrupt target prevents every upload', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'demi-publish-'))
  try {
    const fixture = await release(directory)
    const store = new MemoryStore()
    const target = NATIVE_TARGETS[5]
    const path = join(directory, target, 'commands.exe')
    await writeFile(path, 'corrupt')
    await expect(publishPackages([fixture], store, 'native', new AbortController().signal)).rejects.toThrow('does not match descriptor')
    expect(store.writes).toEqual([])
    await rm(path)
    await expect(publishPackages([fixture], store, 'native', new AbortController().signal)).rejects.toThrow()
    expect(store.writes).toEqual([])
  } finally { await rm(directory, { recursive: true, force: true }) }
})

test('a development release of fewer targets is not published', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'demi-publish-'))
  try {
    const fixture = await release(directory, NATIVE_TARGETS.slice(0, 2))
    const store = new MemoryStore()
    await expect(publishPackages([fixture], store, 'native', new AbortController().signal))
      .rejects.toThrow(`lacks a target and cannot be published: ${NATIVE_TARGETS[2]}`)
    expect(store.writes).toEqual([])
  } finally { await rm(directory, { recursive: true, force: true }) }
})

test('S3 adapter produces HTTPS signed GET URLs without contacting storage', async () => {
  const signal = new AbortController().signal
  const s3 = new S3ArtifactStore('fixture', { region: 'us-east-1', credentials: { accessKeyId: 'fixture', secretAccessKey: 'fixture' } })
  try {
    const s3Url = new URL(await s3.signGet('native/blobs/fixture', 300, signal))
    expect(s3Url.protocol).toBe('https:')
    expect(s3Url.searchParams.get('X-Amz-Expires')).toBe('300')
  } finally {
    s3.close()
  }
})
