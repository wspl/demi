/**
 * A `FileBrowserSource` over an in-memory tree, for prototypes and gallery specimens.
 * Nothing here touches a real disk; a directory can be marked as failing to exercise
 * the browser's error states, and every read can carry a simulated latency.
 */
import {
  FileBrowserError,
  type FileBrowserEntry,
  type FileBrowserFailure,
  type FileBrowserSource,
  type FileBrowserPlatform
} from './types'
import { baseName, joinPath, normalizePath, parentPath } from './paths'

export interface MemoryFile {
  kind: 'file'
  size: number
  modifiedAt: string
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
}

/** Shorthand builders for fixture trees. */
export function dir(
  children: Record<string, MemoryNode>,
  extra: Omit<MemoryDirectory, 'kind' | 'children'> = {}
): MemoryDirectory {
  return { kind: 'directory', children, ...extra }
}

export function file(size: number, modifiedAt: string): MemoryFile {
  return { kind: 'file', size, modifiedAt }
}

export function createMemoryFileSource(options: MemoryFileSourceOptions): FileBrowserSource & {
  root: MemoryDirectory
} {
  const { root, latencyMs = 0 } = options

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
  }
}
