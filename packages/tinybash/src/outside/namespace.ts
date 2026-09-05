import { isAbsolutePath, normalizePath } from '@demicodes/utils'

/** Resolves a path against the cwd and collapses `.` and `..`, POSIX style. */
export function resolvePath(cwd: string, path: string): string {
  return normalizePath(isAbsolutePath(path) ? path : `${cwd}/${path}`)
}

/** Whether a resolved absolute path lies under one of the namespace prefixes (or is `/dev/null`). */
export function insideNamespace(resolved: string, namespace: readonly string[]): boolean {
  if (resolved === '/dev/null') return true
  return namespace.some((prefix) => {
    const root = normalizePath(prefix)
    return resolved === root || resolved.startsWith(root.endsWith('/') ? root : `${root}/`)
  })
}
