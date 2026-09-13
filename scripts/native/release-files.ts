import { createHash, randomUUID } from 'node:crypto'
import { copyFile, mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises'
import { basename, dirname, join } from 'node:path'
import { NATIVE_TARGETS, type NativeArtifact } from '@demicodes/command-protocol'

interface ReleaseFile {
  source: string
  relativePath: string
  artifact: NativeArtifact
}

export async function collectReleaseFiles(directory: string, executable: string) {
  const files: ReleaseFile[] = []
  const targets: Record<string, NativeArtifact> = {}
  for (const target of NATIVE_TARGETS) {
    const name = `${executable}${target.includes('windows') ? '.exe' : ''}`
    const source = join(directory, target, 'release', name)
    const bytes = await readFile(source)
    if (bytes.length === 0)
      throw new Error(`Empty native artifact: ${source}`)
    const artifact = { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length }
    targets[target] = artifact
    files.push({ source, relativePath: join(target, name), artifact })
  }
  return { files, targets }
}

/** Publish a complete directory once; reuse requires matching metadata and bytes. */
export async function publishReleaseDirectory(
  directory: string,
  metadataName: string,
  metadata: string,
  files: readonly ReleaseFile[],
): Promise<void> {
  await mkdir(dirname(directory), { recursive: true })
  const stage = await mkdtemp(join(dirname(directory), `.${basename(directory)}-stage-`))
  try {
    for (const file of files) {
      const destination = join(stage, file.relativePath)
      await mkdir(dirname(destination), { recursive: true })
      await copyFile(file.source, destination)
      await verifyArtifact(destination, file.artifact)
    }
    await writeFile(join(stage, metadataName), metadata)
    try {
      await rename(stage, directory)
    } catch (error) {
      if (!(error instanceof Error && 'code' in error
        && ['EEXIST', 'ENOTEMPTY'].includes(String(error.code))))
        throw error
      if (await readFile(join(directory, metadataName), 'utf8') !== metadata)
        throw new Error('Immutable release directory contains different metadata')
      for (const file of files)
        await verifyArtifact(join(directory, file.relativePath), file.artifact)
    }
  } finally {
    await rm(stage, { recursive: true, force: true })
  }
}

export async function writeReleasePointer(path: string, metadata: string): Promise<void> {
  const temporary = join(dirname(path), `.${basename(path)}-${randomUUID()}`)
  try {
    await writeFile(temporary, metadata)
    await rename(temporary, path)
  } finally {
    await rm(temporary, { force: true })
  }
}

async function verifyArtifact(path: string, expected: NativeArtifact): Promise<void> {
  const bytes = await readFile(path)
  const sha256 = createHash('sha256').update(bytes).digest('hex')
  if (bytes.length !== expected.size || sha256 !== expected.sha256)
    throw new Error(`Native artifact changed or is corrupt: ${path}`)
}
