import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { copyFile, mkdir, readFile, readdir, realpath, rename, rm, stat } from 'node:fs/promises'
import { isAbsolute, join, normalize, relative } from 'node:path'
import { list } from 'tar'
import { createId, errorCode } from '@demicodes/utils'
import { cloudImageManifestSchema } from '../image-manifest'
import { syncFile } from '../machine-image-store'
import type { GVisorConfig } from './config'
import { requireTool } from './image-tools'

/** Verify and import one immutable Cloud root, once per manager startup. */
export async function importBase(config: GVisorConfig): Promise<string> {
  const metadata = await readFile(join(config.image, 'manifest.json'))
  const manifest = cloudImageManifestSchema.parse(JSON.parse(metadata.toString('utf8')))
  const architecture = process.arch === 'arm64' ? 'arm64' : process.arch === 'x64' ? 'amd64' : null
  if (manifest.architecture !== architecture) throw new Error('Cloud image architecture differs from execution host')
  const version = createHash('sha256').update(metadata).digest('hex')
  const parent = join(config.imagesDir, 'bases')
  await mkdir(parent, { recursive: true, mode: 0o700 })
  for (const name of await readdir(parent)) {
    // An import is not visible to a runtime until its final rename.
    if (name.startsWith('.base-')) await rm(join(parent, name), { recursive: true, force: true })
  }
  const target = join(parent, version)
  try {
    const saved = await readFile(join(target, 'manifest.json'))
    if (!saved.equals(metadata)) throw new Error('Pinned Cloud image manifest differs')
    return version
  } catch (error) {
    if (errorCode(error) !== 'ENOENT') throw error
  }
  const stage = join(parent, `.base-${createId()}`)
  const archive = join(stage, manifest.rootfs.file)
  const root = join(stage, 'rootfs')
  await mkdir(root, { recursive: true, mode: 0o700 })
  try {
    await copyFile(join(config.image, manifest.rootfs.file), archive)
    const hash = createHash('sha256')
    for await (const chunk of createReadStream(archive)) hash.update(chunk)
    if ((await stat(archive)).size !== manifest.rootfs.size || hash.digest('hex') !== manifest.rootfs.sha256) {
      throw new Error('Cloud root archive integrity mismatch')
    }
    let invalid: string | undefined
    await list({ file: archive, strict: true, onReadEntry(entry) {
      const path = normalize(entry.path)
      if (isAbsolute(path) || path === '..' || path.startsWith('../') ||
          !['File', 'OldFile', 'Directory', 'SymbolicLink', 'Link'].includes(entry.type)) {
        invalid ??= 'Cloud archive contains an unsafe path or entry type'
      }
      if (entry.type === 'Link' && entry.linkpath) {
        const link = normalize(entry.linkpath)
        if (isAbsolute(link) || link === '..' || link.startsWith('../')) invalid ??= 'Cloud archive contains an unsafe hardlink'
      }
    } })
    if (invalid) throw new Error(invalid)
    // node-tar exposes validated entries but does not restore extended attributes.
    // libarchive supplies that missing extraction capability and secure path handling.
    await requireTool('bsdtar', ['-xpf', archive, '--xattrs', '-C', root], undefined, 300_000)
    for (const [path, expected] of Object.entries(manifest.executables)) {
      const source = await realpath(join(root, path))
      if (relative(root, source).startsWith('..')) throw new Error('Invalid image executable path')
      const digest = createHash('sha256')
      for await (const chunk of createReadStream(source)) digest.update(chunk)
      if ((await stat(source)).size !== expected.size || digest.digest('hex') !== expected.sha256) {
        throw new Error(`Cloud executable integrity mismatch: ${path}`)
      }
    }
    for (const required of ['/usr/bin/demi-runner', '/usr/bin/tini']) {
      if (!manifest.executables[required]) throw new Error(`Cloud image manifest lacks ${required}`)
    }
    await rm(archive)
    await Bun.write(join(stage, 'manifest.json'), metadata)
    await requireTool('sync', ['-f', root])
    await syncFile(stage)
    await rename(stage, target)
    await syncFile(parent)
    return version
  } finally {
    await rm(stage, { recursive: true, force: true })
  }
}
