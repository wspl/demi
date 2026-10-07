/**
 * A `FileBrowserSource` over an in-memory tree, for prototypes and gallery specimens.
 * Nothing here touches a real disk; a directory can be marked as failing to exercise
 * the file browser's error states, every read can carry a simulated latency, and a
 * source given an upload rate takes uploads at that rate, each landing as a
 * file of its size; without one it takes no uploads. Directories are made
 * and entries deleted at once.
 *
 * The source keeps what it read as the product's does (`keptSource`), under a
 * simulated watch of the tree: a live watch confirms what is read, so a file
 * shown again shows at once, and a change made with `change` is reported, so
 * the views that show it read it again; an unavailable one says so, as a Host
 * that cannot watch does.
 */
import { reactive } from 'vue'
import { previewMediaType, TEXT_FILE_BYTES } from '@demicodes/protocol'
import { baseName, delay } from '@demicodes/utils'
import { HostFiles, type Coverage } from './file-cache'
import { keptSource, type FileFollower, type FileReads } from './kept-source'
import {
  FileBrowserError,
  type FileBrowserEntry,
  type FileBrowserFailure,
  type FileBrowserSource,
  type FileBrowserPlatform,
  type FileContents,
  type FileWatchNote,
} from './types'
import { joinPath, normalizePath, parentPath } from './paths'

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
  /** Bytes a second an upload moves at; without it the source takes no uploads. */
  uploadRate?: number
  /**
   * The simulated watch of the tree: `live`, the default, confirms what is
   * read and reports each `change`; `unavailable` is a Host that cannot
   * watch, whose views say they show files as last read.
   */
  watch?: 'live' | 'unavailable'
}

/** What a simulated Host says when it cannot watch its files. */
export const UNWATCHED_REASON = 'The file system reports no changes.'

/** A memory source, with its tree and what simulates the world changing it. */
export interface MemoryFileSource extends FileBrowserSource {
  root: MemoryDirectory
  read(path: string): Promise<string>
  contents: FileContents
  /**
   * Writes `content` to the file at `path` as something outside the page
   * would, making it if missing; a live watch reports it.
   */
  change(path: string, content: string): void
  /** The next read of `path` fails with `failure`, as a Host's might for a moment. */
  failNext(path: string, failure: FileBrowserFailure): void
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

/** A memory tree's reads and writes, each answering as a Host's would, and what changes the tree. */
interface MemoryHost {
  reads: FileReads
  /** The directory at `path`, made down from the root with each one that is missing. */
  makeDirectories(path: string): MemoryDirectory
  /** Paths whose next read fails, as a Host's might for a moment. */
  failing: Map<string, FileBrowserFailure>
}

/** The reads and writes of a memory tree, each a simulated request, without keeping what they answer. */
export function memoryFileReads(options: MemoryFileSourceOptions): FileReads {
  return memoryHost(options).reads
}

function memoryHost(options: MemoryFileSourceOptions): MemoryHost {
  const { root, latencyMs = 0, uploadRate } = options
  const failing = new Map<string, FileBrowserFailure>()

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

  /** Waits a read's latency; then a path told to fail next fails, once. */
  async function wait(path?: string) {
    if (latencyMs > 0)
      await delay(latencyMs)
    const target = path === undefined ? undefined : normalizePath(path)
    const failure = target === undefined ? undefined : failing.get(target)
    if (target !== undefined && failure) {
      failing.delete(target)
      throw new FileBrowserError(failure.kind, failure.message)
    }
  }

  function fileAt(path: string): MemoryFile {
    if (options.offline)
      throw new FileBrowserError('offline')
    const node = lookup(path)
    if (!node || node.kind !== 'file')
      throw new FileBrowserError('not-found', `No such file: ${normalizePath(path)}`)
    return node
  }

  /** The directory at `path`, made down from the root with each one that is missing. */
  function makeDirectories(path: string): MemoryDirectory {
    let node: MemoryDirectory = root
    let at = '/'
    for (const segment of normalizePath(path).split('/').filter(Boolean)) {
      at = joinPath(at, segment)
      const child: MemoryNode | undefined = node.children[segment]
      if (child?.kind === 'file')
        throw new FileBrowserError('other', `${at} is a file.`)
      if (child) {
        node = child
        continue
      }
      if (node.failure?.kind === 'permission')
        throw new FileBrowserError('permission', node.failure.message)
      const made: MemoryDirectory = { kind: 'directory', children: {}, modifiedAt: new Date().toISOString() }
      node.children[segment] = made
      node = made
    }
    return node
  }

  /** Uploads at `rate` bytes a second, each landing as a file of its size. */
  function uploadAt(rate: number): NonNullable<FileBrowserSource['upload']> {
    return async (path, file, { replace, signal, progress }) => {
      if (options.offline)
        throw new FileBrowserError('offline')
      // The write makes the folders above it, as a Host's does.
      const parent = makeDirectories(parentPath(path))
      // A directory that cannot be listed cannot be written to either.
      if (parent.failure)
        throw new FileBrowserError(parent.failure.kind, parent.failure.message)
      const name = baseName(path)
      const taken = parent.children[name]
      if (taken?.kind === 'directory')
        throw new FileBrowserError('other', `${normalizePath(path)} is a folder.`)
      if (taken && !replace)
        throw new FileBrowserError('exists', `${normalizePath(path)} already exists.`)
      let sent = 0
      while (sent < file.size) {
        await delay(UPLOAD_TICK_MS, signal)
        signal.throwIfAborted()
        sent = Math.min(file.size, sent + rate * UPLOAD_TICK_MS / 1000)
        progress(sent)
      }
      // The bytes stay with the page: the file lands as its size and time.
      parent.children[name] = { kind: 'file', size: file.size, modifiedAt: new Date().toISOString() }
    }
  }

  const reads: FileReads = {
    platform: options.platform,
    home: normalizePath(options.home),
    async list(path) {
      await wait(path)
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
    async readText(path, held) {
      await wait(path)
      const node = fileAt(path)
      if (node.url !== undefined && node.content === undefined)
        throw new FileBrowserError('binary', 'The file is not UTF-8 text')
      // The text route's limit, as a Host's text route answers a larger file.
      if (node.size > TEXT_FILE_BYTES)
        throw new FileBrowserError('too-large')
      // A file's version is when it was written.
      return held === node.modifiedAt ? null : { text: node.content ?? '', version: node.modifiedAt }
    },
    /** An asset's own URL, or a text file's content as a `data:` URL typed by its extension. */
    contents: {
      url(path) {
        const node = lookup(path)
        if (!node || node.kind !== 'file')
          return ''
        return node.url ?? `data:${previewMediaType(path) ?? 'text/plain'};charset=utf-8,${encodeURIComponent(node.content ?? '')}`
      },
      async describe(path) {
        await wait(path)
        const node = fileAt(path)
        return { size: node.size, modifiedAt: node.modifiedAt, version: node.modifiedAt }
      },
      async readStart(path, length) {
        await wait(path)
        const node = fileAt(path)
        if (node.content === undefined)
          throw new FileBrowserError('binary', 'The file is not UTF-8 text')
        return new TextEncoder().encode(node.content).subarray(0, length)
      },
    },
    async createDirectory(path) {
      await wait()
      if (options.offline)
        throw new FileBrowserError('offline')
      makeDirectories(path)
    },
    async remove(path) {
      await wait()
      if (options.offline)
        throw new FileBrowserError('offline')
      const parent = lookup(parentPath(path))
      const name = baseName(path)
      if (!parent || parent.kind !== 'directory' || !parent.children[name])
        return
      // A directory that cannot be listed cannot be changed either.
      if (parent.failure)
        throw new FileBrowserError(parent.failure.kind, parent.failure.message)
      delete parent.children[name]
    },
    ...(uploadRate === undefined ? {} : { upload: uploadAt(uploadRate) }),
  }
  return { reads, makeDirectories, failing }
}

export function createMemoryFileSource(options: MemoryFileSourceOptions): MemoryFileSource {
  const { root, watch = 'live' } = options
  const { reads, makeDirectories, failing } = memoryHost(options)
  // The simulated watch covers the whole tree while it is live.
  const files = new HostFiles()
  const everything: Coverage = { covers: () => true }
  if (watch === 'live')
    files.cover(everything)
  const note: FileWatchNote = reactive({
    unavailable: watch === 'unavailable' ? UNWATCHED_REASON : null,
    refresh: () => files.refresh(),
  })
  const follower: FileFollower = { show: () => () => {}, note }
  const source = keptSource(reads, { files, follower })
  const { contents, read } = source
  if (!contents || !read)
    throw new Error('A memory source reads its files and serves their bytes.')
  return Object.assign(source, {
    root,
    read,
    contents,
    change(path: string, content: string) {
      const target = normalizePath(path)
      const parent = makeDirectories(parentPath(target))
      parent.children[baseName(target)] = textFile(content, new Date().toISOString())
      if (watch === 'live')
        files.changed([target])
    },
    failNext(path: string, failure: FileBrowserFailure) {
      failing.set(normalizePath(path), failure)
    },
  })
}
