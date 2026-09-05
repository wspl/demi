import { chmod, mkdir, utimes, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { BlobStore } from '@demicodes/agent'
import type { VirtualFsBackend } from '@demicodes/host-virtual'
import type { HostDirent, HostFileStat } from '@demicodes/shell'
import { basenamePath, concatBytes, dirnamePath, errnoError, normalizePath, SerialQueue } from '@demicodes/utils'
import type { SqlDatabase } from './database'

/**
 * A row of the `files` table: the hostless filesystem's tree (`storage.md`
 * § The hostless filesystem and the home image). Paths are virtual-absolute
 * and normalized; a file's bytes live in the blob store under `sha256`.
 * The root `/` is implicit. No symlinks: nothing hostless can create one.
 */
interface FileRow {
  path: string
  parent: string
  kind: 'file' | 'dir'
  mode: number
  /** Milliseconds since the epoch. */
  mtime: number
  size: number
  sha256: string | null
}

const FILE_MODE = 0o644
const DIR_MODE = 0o755
const SELECT = 'SELECT path, parent, kind, mode, mtime, size, sha256 FROM files'

/**
 * The `VirtualFsBackend` over a conversation's `files` table and the blob
 * store: what `VirtualHost` serves to tinybash and the root commands before
 * the conversation has a machine. Copying a file copies a row; the quota
 * counts the bytes the rows reference.
 */
export function filesTreeBackend(db: SqlDatabase, blobs: BlobStore): VirtualFsBackend {
  const appends = new Map<string, SerialQueue>()
  const row = (path: string): FileRow | null => (path === '/' ? ROOT : db.get<FileRow>(`${SELECT} WHERE path = ?`, [path]))
  const require = (path: string): FileRow => {
    const found = row(path)
    if (!found) throw errnoError('ENOENT', `${path}: no such file or directory`, { path })
    return found
  }
  const requireDir = (path: string): FileRow => {
    const found = require(path)
    if (found.kind !== 'dir') throw errnoError('ENOTDIR', `${path}: not a directory`, { path })
    return found
  }
  const requireFile = (path: string): FileRow => {
    const found = require(path)
    if (found.kind !== 'file') throw errnoError('EISDIR', `${path}: is a directory`, { path })
    return found
  }
  const children = (path: string): FileRow[] => db.all<FileRow>(`${SELECT} WHERE parent = ? ORDER BY path`, [path])
  const subtree = (path: string): FileRow[] => db.all<FileRow>(`${SELECT} WHERE path = ? OR path LIKE ? ESCAPE '!' ORDER BY path`, [path, `${escapeLike(path)}/%`])
  const upsert = (entry: FileRow): void => {
    db.run(
      'INSERT INTO files (path, parent, kind, mode, mtime, size, sha256) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (path) DO UPDATE SET parent = excluded.parent, kind = excluded.kind, mode = excluded.mode, mtime = excluded.mtime, size = excluded.size, sha256 = excluded.sha256',
      [entry.path, entry.parent, entry.kind, entry.mode, entry.mtime, entry.size, entry.sha256],
    )
  }
  const touchParent = (path: string): void => {
    const parent = dirnamePath(path)
    if (parent !== '/') db.run('UPDATE files SET mtime = ? WHERE path = ?', [Date.now(), parent])
  }
  const mkdirs = (path: string, recursive: boolean): void => {
    if (path === '/') {
      if (!recursive) throw errnoError('EEXIST', `${path}: file exists`, { path })
      return
    }
    const existing = row(path)
    if (existing) {
      if (recursive && existing.kind === 'dir') return
      throw errnoError('EEXIST', `${path}: file exists`, { path })
    }
    const parent = dirnamePath(path)
    if (recursive) mkdirs(parent, true)
    else requireDir(parent)
    const now = Date.now()
    upsert({ path, parent, kind: 'dir', mode: DIR_MODE, mtime: now, size: 0, sha256: null })
    touchParent(path)
  }
  const writeBytes = async (path: string, data: Uint8Array, createParents: boolean): Promise<void> => {
    if (path === '/') throw errnoError('EISDIR', `${path}: is a directory`, { path })
    const existing = row(path)
    if (existing?.kind === 'dir') throw errnoError('EISDIR', `${path}: is a directory`, { path })
    const parent = dirnamePath(path)
    const parentRow = row(parent)
    if (!parentRow) {
      if (!createParents) throw errnoError('ENOENT', `${parent}: no such file or directory`, { path })
    } else if (parentRow.kind !== 'dir') throw errnoError('ENOTDIR', `${parent}: not a directory`, { path })
    const sha256 = await blobs.put(data)
    db.transaction(() => {
      if (!parentRow) mkdirs(parent, true)
      upsert({ path, parent, kind: 'file', mode: existing?.mode ?? FILE_MODE, mtime: Date.now(), size: data.byteLength, sha256 })
      if (!existing) touchParent(path)
    })
  }
  const bytesOf = async (entry: FileRow): Promise<Uint8Array> => {
    const data = entry.sha256 === null ? null : await blobs.get(entry.sha256)
    if (!data) throw errnoError('EIO', `${entry.path}: its bytes are missing from the blob store`, { path: entry.path })
    return data
  }
  const remove = (path: string, recursive: boolean, force: boolean): void => {
    const existing = row(path)
    if (!existing) {
      if (force) return
      throw errnoError('ENOENT', `${path}: no such file or directory`, { path })
    }
    if (path === '/') throw errnoError('EPERM', `${path}: cannot remove the root`, { path })
    if (existing.kind === 'dir') {
      if (!recursive) {
        if (children(path).length > 0) throw errnoError('ENOTEMPTY', `${path}: directory not empty`, { path })
      }
      db.run("DELETE FROM files WHERE path = ? OR path LIKE ? ESCAPE '!'", [path, `${escapeLike(path)}/%`])
    } else {
      db.run('DELETE FROM files WHERE path = ?', [path])
    }
    touchParent(path)
  }
  const copy = (source: FileRow, destination: string, recursive: boolean): void => {
    if (source.kind === 'dir' && !recursive) throw errnoError('EISDIR', `${source.path}: is a directory (cp needs recursive)`, { path: source.path })
    if (destination === source.path || destination.startsWith(`${source.path}/`)) {
      throw errnoError('EINVAL', `${destination}: cannot copy ${source.path} into itself`, { path: destination })
    }
    const target = row(destination)
    if (target?.kind === 'dir' && source.kind === 'file') {
      throw errnoError('EISDIR', `${destination}: is a directory`, { path: destination })
    }
    const parent = dirnamePath(destination)
    requireDir(parent)
    const now = Date.now()
    db.transaction(() => {
      if (source.kind === 'dir') {
        if (target && target.kind !== 'dir') throw errnoError('ENOTDIR', `${destination}: not a directory`, { path: destination })
        for (const entry of subtree(source.path)) {
          const path = entry.path === source.path ? destination : `${destination}${entry.path.slice(source.path.length)}`
          upsert({ ...entry, path, parent: dirnamePath(path), mtime: entry.kind === 'dir' ? now : entry.mtime })
        }
      } else {
        upsert({ ...source, path: destination, parent, mtime: now })
      }
      touchParent(destination)
    })
  }
  const move = (source: FileRow, destination: string): void => {
    if (destination === source.path) return
    if (destination.startsWith(`${source.path}/`)) throw errnoError('EINVAL', `${destination}: cannot move ${source.path} into itself`, { path: destination })
    const target = row(destination)
    if (target) {
      if (target.kind === 'dir' && source.kind !== 'dir') throw errnoError('EISDIR', `${destination}: is a directory`, { path: destination })
      if (target.kind !== 'dir' && source.kind === 'dir') throw errnoError('ENOTDIR', `${destination}: not a directory`, { path: destination })
      if (target.kind === 'dir' && children(destination).length > 0) throw errnoError('ENOTEMPTY', `${destination}: directory not empty`, { path: destination })
    }
    requireDir(dirnamePath(destination))
    db.transaction(() => {
      if (target) db.run('DELETE FROM files WHERE path = ?', [destination])
      for (const entry of subtree(source.path)) {
        const path = entry.path === source.path ? destination : `${destination}${entry.path.slice(source.path.length)}`
        db.run('UPDATE files SET path = ?, parent = ? WHERE path = ?', [path, dirnamePath(path), entry.path])
      }
      touchParent(source.path)
      touchParent(destination)
    })
  }

  return {
    usage: async () => db.get<{ total: number }>("SELECT COALESCE(SUM(size), 0) AS total FROM files WHERE kind = 'file'")?.total ?? 0,
    readFile: async (path) => bytesOf(requireFile(normalizePath(path))),
    writeFile: (path, data, options) => writeBytes(normalizePath(path), data, options?.createParents ?? false),
    appendFile: async (path, data, options) => {
      const target = normalizePath(path)
      let queue = appends.get(target)
      if (!queue) {
        queue = new SerialQueue()
        appends.set(target, queue)
      }
      try {
        await queue.run(async () => {
          const existing = row(target)
          if (existing?.kind === 'file') return writeBytes(target, concatBytes([await bytesOf(existing), data]), false)
          return writeBytes(target, data, options?.createParents ?? false)
        })
      } finally {
        if (queue.idle) appends.delete(target)
      }
    },
    exists: async (path) => row(normalizePath(path)) !== null,
    stat: async (path) => statOf(require(normalizePath(path))),
    lstat: async (path) => statOf(require(normalizePath(path))),
    readdir: (async (path: string, options?: { withFileTypes?: boolean }) => {
      const entries = children(requireDir(normalizePath(path)).path)
      if (options?.withFileTypes) return entries.map(direntOf)
      return entries.map((entry) => basenamePath(entry.path))
    }) as VirtualFsBackend['readdir'],
    mkdir: async (path, options) => mkdirs(normalizePath(path), options?.recursive ?? false),
    rm: async (path, options) => remove(normalizePath(path), options?.recursive ?? false, options?.force ?? false),
    cp: async (path, destination, options) => copy(require(normalizePath(path)), normalizePath(destination), options?.recursive ?? false),
    mv: async (path, destination) => move(require(normalizePath(path)), normalizePath(destination)),
    chmod: async (path, mode) => {
      const target = require(normalizePath(path))
      db.run('UPDATE files SET mode = ? WHERE path = ?', [mode & 0o7777, target.path])
    },
    symlink: async (_target, path) => {
      throw errnoError('EPERM', `${path}: the hostless filesystem holds no symbolic links`, { path })
    },
    link: async (_existingPath, path) => {
      throw errnoError('EPERM', `${path}: the hostless filesystem holds no hard links`, { path })
    },
    readlink: async (path) => {
      throw errnoError('EINVAL', `${path}: not a symbolic link`, { path })
    },
    realpath: async (path) => require(normalizePath(path)).path,
    utimes: async (path, _atime, mtime) => {
      const target = require(normalizePath(path))
      db.run('UPDATE files SET mtime = ? WHERE path = ?', [mtime.getTime(), target.path])
    },
  }
}

/** One subtree of the tree written under one real directory: `/home/demi` → `<dir>`, `/tmp` → `<dir>/.tmp`. */
export interface TreePlacement {
  from: string
  to: string
}

/**
 * Writes the tree into real directories, modes and mtimes included: the
 * directory a home image is built from (`storage.md` § The hostless
 * filesystem and the home image). Directories are created for every
 * placement even when the subtree is empty.
 */
export async function materializeFilesTree(db: SqlDatabase, blobs: BlobStore, placements: readonly TreePlacement[]): Promise<void> {
  for (const placement of placements) {
    await mkdir(placement.to, { recursive: true })
    const rows = db.all<FileRow>(`${SELECT} WHERE path LIKE ? ESCAPE '!' ORDER BY path`, [`${escapeLike(placement.from)}/%`])
    for (const entry of rows) {
      const real = join(placement.to, entry.path.slice(placement.from.length + 1))
      if (entry.kind === 'dir') {
        await mkdir(real, { recursive: true })
      } else {
        const data = entry.sha256 === null ? null : await blobs.get(entry.sha256)
        if (!data) throw new Error(`${entry.path}: its bytes are missing from the blob store`)
        await writeFile(real, data)
      }
      await chmod(real, entry.mode & 0o7777)
    }
    // Times last, deepest first: writing a child would bump its parent's mtime again.
    for (const entry of [...rows].reverse()) {
      const real = join(placement.to, entry.path.slice(placement.from.length + 1))
      await utimes(real, new Date(entry.mtime), new Date(entry.mtime))
    }
  }
}

/** The tree rows are deleted once the conversation has a home image; the blobs stay. */
export function clearFilesTree(db: SqlDatabase): void {
  db.run('DELETE FROM files')
}

const ROOT: FileRow = { path: '/', parent: '', kind: 'dir', mode: DIR_MODE, mtime: 0, size: 0, sha256: null }

function statOf(entry: FileRow): HostFileStat {
  return {
    isFile: entry.kind === 'file',
    isDirectory: entry.kind === 'dir',
    isSymbolicLink: false,
    mode: entry.mode | (entry.kind === 'dir' ? 0o040000 : 0o100000),
    size: entry.size,
    mtime: new Date(entry.mtime),
    uid: 1000,
    gid: 1000,
    nlink: 1,
  }
}

function direntOf(entry: FileRow): HostDirent {
  return { name: basenamePath(entry.path), isFile: entry.kind === 'file', isDirectory: entry.kind === 'dir', isSymbolicLink: false }
}

function escapeLike(value: string): string {
  return value.replace(/[!%_]/g, (char) => `!${char}`)
}
