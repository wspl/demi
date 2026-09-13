import type { HostFileSystem } from '@demicodes/shell'
import { decodeUtf8Strict, errorCode } from '@demicodes/utils'

/** The most a file route sends as text (`web-api.md` § File text and working tree changes). */
export const TEXT_FILE_MAX_BYTES = 4 * 1024 * 1024

export class TextFileRefused extends Error {
  readonly code: 'file_too_large' | 'not_text'

  constructor(code: 'file_too_large' | 'not_text') {
    super(code === 'file_too_large' ? 'The file is too large to show' : 'The file is not UTF-8 text')
    this.name = 'TextFileRefused'
    this.code = code
  }
}

/** UTF-8 text from bytes; `TextFileRefused` beyond the size limit or for other encodings. */
export function textOf(bytes: Uint8Array): string {
  if (bytes.byteLength > TEXT_FILE_MAX_BYTES)
    throw new TextFileRefused('file_too_large')
  const text = decodeUtf8Strict(bytes)
  if (text === null)
    throw new TextFileRefused('not_text')
  return text
}

/** One file of the device as text, under the same limits as `textOf`. */
export async function readTextFile(fs: HostFileSystem, path: string): Promise<string> {
  const stat = await fs.stat(path)
  if (stat.size > TEXT_FILE_MAX_BYTES)
    throw new TextFileRefused('file_too_large')
  return textOf(await fs.readFile(path))
}

/**
 * File metadata comes from the selected runner. A disappeared entry is omitted
 * from this directory snapshot.
 */
export async function browseDirectory(fs: HostFileSystem, path: string) {
  const entries = await fs.readdir(path, { withFileTypes: true })
  const result = []
  // Await each metadata reply so one listing cannot flood the runner's queue.
  for (const entry of entries) {
    const absolute = `${path.replace(/\/+$/, '')}/${entry.name}`
    try {
      const stat = await fs.lstat(absolute)
      result.push({
        name: entry.name,
        isDirectory: entry.isDirectory,
        isSymbolicLink: stat.isSymbolicLink,
        size: stat.size,
        modifiedAt: stat.mtime.toISOString(),
      })
    } catch (error) {
      if (errorCode(error) !== 'ENOENT') {
        throw error
      }
      // A file removed after readdir no longer belongs in this snapshot.
    }
  }
  return result
}
