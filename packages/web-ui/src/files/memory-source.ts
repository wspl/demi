/**
 * A `FileBrowserSource` over an in-memory tree, for prototypes and gallery specimens.
 * Nothing here touches a real disk; a directory can be marked as failing to exercise
 * the browser's error states, every read can carry a simulated latency, and an
 * upload moves at a simulated rate, landing as a file of its size.
 */
import { previewMediaType } from '@demicodes/core'
import { delay } from '@demicodes/utils'
import {
  FileBrowserError,
  type FileBrowserEntry,
  type FileBrowserFailure,
  type FileBrowserSource,
  type FileBrowserPlatform,
  type FileContents
} from './types'
import { baseName, joinPath, normalizePath, parentPath } from './paths'

export interface MemoryFile {
  kind: 'file'
  size: number
  modifiedAt: string
  /** What `read` returns; a file without it reads as empty. */
  content?: string
  /** Where a binary file's bytes load from, a fixture asset; `read` refuses such a file as binary. */
  url?: string
}

export interface MemoryDirectory {
  kind: 'directory'
  modifiedAt?: string
  /** Listing this directory fails with this failure instead of returning its children. */
  failure?: FileBrowserFailure
  children: Record<string, MemoryNode>
}

export type MemoryNode = MemoryFile | MemoryDirectory

export interface MemoryFileSourceOptions {
  platform: FileBrowserPlatform
  home: string
  root: MemoryDirectory
  /** Milliseconds each read takes; a real host is never instant. */
  latencyMs?: number
  /** Every read rejects with this: the whole device is unreachable. */
  offline?: boolean
  /** Bytes a second an upload moves at; a real link is never instant. */
  uploadRate?: number
}

/** How often a simulated upload reports its progress. */
const UPLOAD_TICK_MS = 100

/** Shorthand builders for fixture trees. */
export function dir(
  children: Record<string, MemoryNode>,
  extra: Omit<MemoryDirectory, 'kind' | 'children'> = {}
): MemoryDirectory {
  return { kind: 'directory', children, ...extra }
}

export function file(size: number, modifiedAt: string, content?: string): MemoryFile {
  return content === undefined
    ? { kind: 'file', size, modifiedAt }
    : { kind: 'file', size, modifiedAt, content }
}

/** A file whose size is its text's length. */
export function textFile(content: string, modifiedAt: string): MemoryFile {
  return { kind: 'file', size: content.length, modifiedAt, content }
}

/** A binary file whose bytes are a fixture asset at `url`. */
export function assetFile(url: string, size: number, modifiedAt: string): MemoryFile {
  return { kind: 'file', size, modifiedAt, url }
}

export function createMemoryFileSource(options: MemoryFileSourceOptions): FileBrowserSource & {
  root: MemoryDirectory
  contents: FileContents
} {
  const { root, latencyMs = 0, uploadRate = 24 * 1024 * 1024 } = options

  function lookup(path: string): MemoryNode | null {
    let node: MemoryNode = root
    for (const segment of normalizePath(path).split('/').filter(Boolean)) {
      if (node.kind !== 'directory')
        return null
      const child: MemoryNode | undefined = node.children[segment]
      if (!child)
        return null
      node = child
    }
    return node
  }

  async function wait(signal?: AbortSignal) {
    if (latencyMs <= 0)
      return
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(resolve, latencyMs)
      signal?.addEventListener('abort', () => {
        clearTimeout(timer)
        reject(new DOMException('Aborted', 'AbortError'))
      }, { once: true })
    })
  }

  function fileAt(path: string): MemoryFile {
    if (options.offline)
      throw new FileBrowserError('offline')
    const node = lookup(path)
    if (!node || node.kind !== 'file')
      throw new FileBrowserError('not-found', `No such file: ${normalizePath(path)}`)
    return node
  }

  /** An asset's own URL, or a text file's content as a `data:` URL typed by its extension. */
  const contents: FileContents = {
    url(path) {
      const node = lookup(path)
      if (!node || node.kind !== 'file')
        return ''
      return node.url ?? `data:${previewMediaType(path) ?? 'text/plain'};charset=utf-8,${encodeURIComponent(node.content ?? '')}`
    },
    async describe(path, signal) {
      await wait(signal)
      const node = fileAt(path)
      return { size: node.size, modifiedAt: node.modifiedAt, version: node.modifiedAt }
    },
  }

  return {
    root,
    platform: options.platform,
    home: normalizePath(options.home),
    async list(path, signal) {
      await wait(signal)
      if (options.offline)
        throw new FileBrowserError('offline')
      const node = lookup(path)
      if (!node || node.kind !== 'directory')
        throw new FileBrowserError('not-found', `No such directory: ${normalizePath(path)}`)
      if (node.failure)
        throw new FileBrowserError(node.failure.kind, node.failure.message)
      const entries: FileBrowserEntry[] = Object.entries(node.children).map(([
        name,
        child
      ]) =>
        child.kind === 'directory'
          ? { name, isDirectory: true, modifiedAt: child.modifiedAt }
          : {
            name,
            isDirectory: false,
            size: child.size,
            modifiedAt: child.modifiedAt
          },
      )
      return entries
    },
    async read(path, signal) {
      await wait(signal)
      const node = fileAt(path)
      if (node.url !== undefined && node.content === undefined)
        throw new FileBrowserError('binary', 'The file is not UTF-8 text')
      return node.content ?? ''
    },
    contents,
    async createDirectory(path) {
      await wait()
      if (options.offline)
        throw new FileBrowserError('offline')
      const parent = lookup(parentPath(path))
      if (!parent || parent.kind !== 'directory')
        throw new FileBrowserError('not-found', `No such directory: ${parentPath(path)}`)
      if (parent.failure?.kind === 'permission')
        throw new FileBrowserError('permission', parent.failure.message)
      const name = baseName(path)
      if (parent.children[name])
        throw new FileBrowserError(
        'other',
        `${joinPath(parentPath(path), name)} already exists.`
      )
      parent.children[name] = {
        kind: 'directory',
        children: {},
        modifiedAt: new Date().toISOString()
      }
    },
    async upload(path, file, { replace, signal, progress }) {
      if (options.offline)
        throw new FileBrowserError('offline')
      const parent = lookup(parentPath(path))
      if (!parent || parent.kind !== 'directory')
        throw new FileBrowserError('not-found', `No such directory: ${parentPath(path)}`)
      // A directory that cannot be listed cannot be written to either.
      if (parent.failure)
        throw new FileBrowserError(parent.failure.kind, parent.failure.message)
      const name = baseName(path)
      if (parent.children[name] && !replace)
        throw new FileBrowserError('exists', `${normalizePath(path)} already exists.`)
      let sent = 0
      while (sent < file.size) {
        await delay(UPLOAD_TICK_MS, signal)
        signal.throwIfAborted()
        sent = Math.min(file.size, sent + uploadRate * UPLOAD_TICK_MS / 1000)
        progress(sent)
      }
      // The bytes stay with the page: the file lands as its size and time.
      parent.children[name] = { kind: 'file', size: file.size, modifiedAt: new Date().toISOString() }
    },
  }
}
