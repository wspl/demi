import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import type { NativeArtifact } from '@demicodes/command-protocol'

/** Measure the exact bytes published or installed as a native release artifact. */
export async function fileArtifact(path: string): Promise<NativeArtifact> {
  const bytes = await readFile(path)
  if (bytes.length === 0) throw new Error(`Empty native artifact: ${path}`)
  return { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length }
}
