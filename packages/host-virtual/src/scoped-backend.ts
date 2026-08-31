import type { HostFileSystem } from '@demicodes/shell'
import { isAbsolutePath, normalizePath } from '@demicodes/utils'
import type { VirtualFsBackend } from './virtual-host'

/**
 * A `VirtualFsBackend` over a real directory: virtual-absolute paths map to
 * `<realRoot>/<path>` through any `HostFileSystem` (the backend product uses a
 * `LocalHost`'s). Full fs semantics — symlinks, hardlinks, chmod — with the
 * real root never leaking back out: absolute symlink targets are translated on
 * write and untranslated on read, and `realpath` answers in virtual terms
 * (resolutions that leave the root fail).
 */
export function scopedFsBackend(realRoot: string, fs: HostFileSystem): VirtualFsBackend {
  const root = normalizePath(realRoot)

  const toReal = (virtualPath: string): string => {
    const normalized = normalizePath(virtualPath)
    return normalized === '/' ? root : `${root}${normalized}`
  }

  // The real root itself may sit behind symlinks (macOS /var → /private/var),
  // so canonical-path comparisons use its resolved form, computed lazily.
  let canonicalRoot: Promise<string> | null = null
  const resolveRoot = (): Promise<string> => {
    canonicalRoot ??= fs.realpath(root).then(normalizePath, () => root)
    return canonicalRoot
  }

  const fromReal = async (realPath: string): Promise<string> => {
    const normalized = normalizePath(realPath)
    for (const base of [root, await resolveRoot()]) {
      if (normalized === base) return '/'
      if (normalized.startsWith(`${base}/`)) return normalized.slice(base.length)
    }
    throw Object.assign(new Error(`${realPath}: resolves outside the virtual workspace`), { code: 'EPERM' })
  }

  return {
    readFile: (path) => fs.readFile(toReal(path)),
    writeFile: (path, data, options) => fs.writeFile(toReal(path), data, options),
    appendFile: (path, data, options) => fs.appendFile(toReal(path), data, options),
    exists: (path) => fs.exists(toReal(path)),
    stat: (path) => fs.stat(toReal(path)),
    lstat: (path) => fs.lstat(toReal(path)),
    readdir: ((path: string, options?: { withFileTypes?: boolean }) =>
      options?.withFileTypes ? fs.readdir(toReal(path), { withFileTypes: true }) : fs.readdir(toReal(path))) as HostFileSystem['readdir'],
    mkdir: (path, options) => fs.mkdir(toReal(path), options),
    rm: (path, options) => fs.rm(toReal(path), options),
    cp: (path, destination, options) => fs.cp(toReal(path), toReal(destination), options),
    mv: (path, destination) => fs.mv(toReal(path), toReal(destination)),
    chmod: (path, mode) => fs.chmod(toReal(path), mode),
    symlink: (target, path) => fs.symlink(isAbsolutePath(target) ? toReal(target) : target, toReal(path)),
    link: (existingPath, path) => fs.link(toReal(existingPath), toReal(path)),
    readlink: async (path) => {
      const target = await fs.readlink(toReal(path))
      return isAbsolutePath(target) ? fromReal(target) : target
    },
    realpath: async (path) => fromReal(await fs.realpath(toReal(path))),
    utimes: (path, atime, mtime) => fs.utimes(toReal(path), atime, mtime),
  }
}
