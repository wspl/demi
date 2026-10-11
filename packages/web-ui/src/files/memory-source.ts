/**
 * A `FileBrowserSource` over an in-memory tree, for prototypes and gallery specimens.
 * Nothing here touches a real disk; a directory can be marked as failing to exercise
 * the file browser's error states, every read can carry a simulated latency, and a
 * source given an upload rate takes uploads at that rate, each landing as a
 * file of its size; without one it takes no uploads. Directories are made
 * and entries deleted at once.
 *
 * The source keeps what it read as the product's does (`keptSource`), with
 * the subscriptions a simulated Host's watch answers (`SimulatedWatch`):
 * each read carries the simulated Host's number, so a file shown again
 * shows at once without a read, and a change made with `change` is
 * reported, so the views that show it read it again; a watch that is
 * unavailable says so for each subscription, as a Host that cannot watch
 * does.
 */
import { previewMediaType, TEXT_FILE_BYTES } from '@demicodes/protocol'
import { baseName, delay } from '@demicodes/utils'
import { HostFiles } from './file-cache'
import { keptSource, type FileReads } from './kept-source'
import {
  FileBrowserError,
  type FileBrowserEntry,
  type FileBrowserFailure,
  type FileBrowserSource,
  type FileBrowserPlatform,
  type FileContents,
} from './types'
import { joinPath, normalizePath, parentPath } from './paths'
import { WatchChannel, type OpenWatchLink, type ReadAt, type Subject, type WatchLinkHandlers } from './watch-channel'

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
   * The simulated watch of the tree: `live`, the default, answers each
   * subscription and reports each `change`; `unavailable` is a Host that
   * cannot watch, whose views say they show files as last read.
   */
  watch?: 'live' | 'unavailable'
}

/** What a simulated Host says when it cannot watch its files. */
export const UNWATCHED_REASON = 'The file system reports no changes.'

/** Whether `path` is `ancestor` or lies below it. */
function within(path: string, ancestor: string): boolean {
  const prefix = ancestor.endsWith('/') ? ancestor : `${ancestor}/`
  return path === ancestor || path.startsWith(prefix)
}

/**
 * A Host's file watch as a page sees it, simulated over a memory tree, for
 * gallery specimens and tests (`runner.md` § Watching files): it counts its
 * changes with one number, answers each subscription `live` from the number
 * now, or `unavailable`, reports a `change` to each subscription it
 * concerns with the change's number, acknowledges each renew, and sends a
 * heartbeat every 30 seconds, as the backend does. A read takes `readAt()`
 * before it reads, as the Host's does.
 */
export class SimulatedWatch {
  private number = 0
  private readonly links = new Set<{ handlers: WatchLinkHandlers; held: Map<string, Subject> }>()

  constructor(private readonly mode: 'live' | 'unavailable' = 'live', private readonly epoch = 'memory') {}

  /** The number now, which a read answers with. */
  readAt(): ReadAt {
    return { epoch: this.epoch, number: this.number }
  }

  /** Opens a connection, which opens in a moment as a socket does. */
  readonly open: OpenWatchLink = (handlers) => {
    const link = { handlers, held: new Map<string, Subject>() }
    let closed = false
    const tell = (message: Parameters<WatchLinkHandlers['message']>[0]) => queueMicrotask(() => {
      if (!closed)
        handlers.message(message)
    })
    // Nothing else may come for long: the watch stays known to be alive.
    const heartbeat = setInterval(() => tell({ type: 'heartbeat' }), 30_000)
    queueMicrotask(() => {
      if (closed)
        return
      this.links.add(link)
      handlers.opened()
    })
    return {
      send: (request) => {
        switch (request.type) {
          case 'subscribe':
            for (const { key, kind, path } of request.subscriptions) {
              if (this.mode === 'unavailable') {
                tell({ type: 'unavailable', key, reason: UNWATCHED_REASON })
                continue
              }
              link.held.set(key, { kind, path })
              tell({ type: 'live', key, since: this.number, epoch: this.epoch })
            }
            return
          case 'unsubscribe':
            for (const key of request.keys)
              link.held.delete(key)
            return
          case 'renew':
            tell({ type: 'renewed', keys: request.keys.filter((key) => link.held.has(key)) })
        }
      },
      close: () => {
        closed = true
        clearInterval(heartbeat)
        this.links.delete(link)
      },
    }
  }

  /**
   * Something outside the page changed `path`: a file written, or with
   * `entry` one that came or went in its folder. Each subscription it
   * concerns hears of it with the change's number.
   */
  change(path: string, { entry = false }: { entry?: boolean } = {}): void {
    this.number += 1
    const at = this.number
    for (const link of this.links) {
      const changes: [string, number][] = []
      for (const [key, subject] of link.held) {
        if (concerns(subject, path, entry))
          changes.push([key, at])
      }
      if (changes.length > 0)
        queueMicrotask(() => link.handlers.message({ type: 'changed', changes }))
    }
  }
}

/** Whether a change of `path` changes a subscription to `subject`; `entry` when an entry came or went. */
function concerns({ kind, path: subject }: Subject, path: string, entry: boolean): boolean {
  switch (kind) {
    case 'file':
      return within(subject, path)
    case 'folder_names':
      return within(subject, path) || (entry && parentPath(path) === subject)
    case 'folder_details':
      return within(subject, path) || parentPath(path) === subject
    case 'worktree':
      return within(path, subject)
    case 'head':
      return false
  }
}

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

/**
 * The reads and writes of a memory tree, each a simulated request, without
 * keeping what they answer; each read carries `watch`'s number, and none
 * without one.
 */
export function memoryFileReads(options: MemoryFileSourceOptions, watch?: SimulatedWatch): FileReads {
  return memoryHost(options, watch).reads
}

function memoryHost(options: MemoryFileSourceOptions, watch?: SimulatedWatch): MemoryHost {
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

  /** The simulated Host's number before a read. */
  const readAt = () => watch?.readAt() ?? null

  function fileAt(path: string, at: ReadAt | null): MemoryFile {
    if (options.offline)
      throw new FileBrowserError('offline')
    const node = lookup(path)
    if (!node || node.kind !== 'file')
      throw new FileBrowserError('not-found', `No such file: ${normalizePath(path)}`, { readAt: at })
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
      const at = readAt()
      await wait(path)
      if (options.offline)
        throw new FileBrowserError('offline')
      const node = lookup(path)
      if (!node || node.kind !== 'directory')
        throw new FileBrowserError('not-found', `No such directory: ${normalizePath(path)}`, { readAt: at })
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
      return { value: entries, readAt: at }
    },
    async readText(path, held) {
      const at = readAt()
      await wait(path)
      const node = fileAt(path, at)
      // A file's version is when it was written.
      const version = node.modifiedAt
      if (node.url !== undefined && node.content === undefined)
        throw new FileBrowserError('binary', 'The file is not UTF-8 text', { version, readAt: at })
      // The text route's limit, as a Host's text route answers a larger file.
      if (node.size > TEXT_FILE_BYTES)
        throw new FileBrowserError('too-large', undefined, { version, readAt: at })
      return held === version
        ? { unchanged: true, readAt: at }
        : { value: { text: node.content ?? '', version }, readAt: at }
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
        const at = readAt()
        await wait(path)
        const node = fileAt(path, at)
        return { value: { size: node.size, modifiedAt: node.modifiedAt, version: node.modifiedAt }, readAt: at }
      },
      async readStart(path, length) {
        await wait(path)
        const node = fileAt(path, null)
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
  const { root } = options
  const host = new SimulatedWatch(options.watch)
  const { reads, makeDirectories, failing } = memoryHost(options, host)
  const files = new HostFiles()
  const source = keptSource(reads, { files, watch: new WatchChannel(files, host.open, 'simulated file watch') })
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
      const entry = parent.children[baseName(target)] === undefined
      parent.children[baseName(target)] = textFile(content, new Date().toISOString())
      host.change(target, { entry })
    },
    failNext(path: string, failure: FileBrowserFailure) {
      failing.set(normalizePath(path), failure)
    },
  })
}
