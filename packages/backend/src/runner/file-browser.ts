import type { HostFileSystem } from '@demicodes/shell'
import { EDIT_FILE_BYTES } from '@demicodes/command-protocol'
import { decodeUtf8Strict, errorCode } from '@demicodes/utils'

/** How far into a file a NUL byte marks it binary; git's probe, and the runner's. */
const BINARY_PROBE_BYTES = 8_000

export class TextFileRefused extends Error {
  readonly code: 'file_too_large' | 'not_text'

  constructor(code: 'file_too_large' | 'not_text') {
    super(code === 'file_too_large' ? 'The file is too large to show' : 'The file is not UTF-8 text')
    this.name = 'TextFileRefused'
    this.code = code
  }
}

/**
 * The text of a file the product shows or retains: a browsed file, a working
 * tree side, a retained edit snapshot. One limit for all of them, the
 * runner's `EDIT_FILE_BYTES` (`web-api.md` § File text and working tree
 * changes). A NUL byte is valid UTF-8 but marks a binary file: the same
 * probe git and the runner's line counting use (`file_diff.rs`), so a file
 * the runner counted lines for is one the browser shows, and no other.
 */
export function textOf(bytes: Uint8Array): string {
  if (bytes.byteLength > EDIT_FILE_BYTES)
    throw new TextFileRefused('file_too_large')
  if (bytes.subarray(0, BINARY_PROBE_BYTES).includes(0))
    throw new TextFileRefused('not_text')
  const text = decodeUtf8Strict(bytes)
  if (text === null)
    throw new TextFileRefused('not_text')
  return text
}

/** One file of the device as text, under the same limits as `textOf`. */
export async function readTextFile(fs: HostFileSystem, path: string): Promise<string> {
  const stat = await fs.stat(path)
  if (stat.size > EDIT_FILE_BYTES)
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
