/** POSIX path arithmetic for the file browser: pure, no filesystem. */
import { baseName } from '@demicodes/utils'

/** Collapses repeated and trailing slashes and resolves `.` and `..`; a relative input is taken from the root. */
export function normalizePath(path: string): string {
  const segments: string[] = []
  for (const segment of path.split('/')) {
    if (segment === '' || segment === '.')
      continue
    if (segment === '..') {
      segments.pop()
      continue
    }
    segments.push(segment)
  }
  return `/${segments.join('/')}`
}

export function joinPath(base: string, name: string): string {
  return normalizePath(`${base}/${name}`)
}

/** A typed path with a leading `~` or `~/` read from `home`, as a shell reads it; any other path as it is. */
export function expandHome(path: string, home: string): string {
  if (path === '~')
    return home
  return path.startsWith('~/') ? `${home}${path.slice(1)}` : path
}

/**
 * The absolute path a person typed into a path field: `~` and `~/…` name
 * the home, an absolute path is itself, and a relative one starts from
 * `root`, or from the filesystem root without one (`file-previews.md` § The
 * File view's bars).
 */
export function typedPath(text: string, places: { home: string; root?: string }): string {
  const expanded = expandHome(text, places.home)
  return normalizePath(places.root === undefined ? expanded : resolveHostPath(places.root, expanded))
}

/**
 * A path as a Host names it: absolute on either path syntax (`/…` or
 * `C:/…`) as it is, relative resolved against `base`. Backslashes count as
 * slashes.
 */
export function resolveHostPath(base: string, path: string): string {
  const normalized = path.replaceAll('\\', '/')
  return normalized.startsWith('/') || /^[A-Za-z]:\//.test(normalized)
    ? normalized
    : joinPath(base, normalized)
}

/**
 * Whether `path` is `folder` or lies within it, compared as the Host names
 * them, on either path syntax.
 */
export function isWithinPath(path: string, folder: string): boolean {
  return path === folder || path.startsWith(`${folder}/`) || path.startsWith(`${folder}\\`)
}

/** The root is its own parent. */
export function parentPath(path: string): string {
  const normalized = normalizePath(path)
  const index = normalized.lastIndexOf('/')
  return index <= 0 ? '/' : normalized.slice(0, index)
}

/** The plugin kit takes its path helpers from here (`plugin-sdk` reaches only `web-ui`). */
export { baseName } from '@demicodes/utils'

/** `path` as it reads from `root`: `a/b` under it, `''` for `root` itself, any other path whole. */
export function relativePath(root: string, path: string): string {
  // A Windows root reads both in Windows' spelling: `\` and `/` separate,
  // and case does not tell names apart.
  const windows = isWindowsPath(root)
  const spelled = (text: string): string => normalizePath(windows ? text.replaceAll('\\', '/') : text)
  const same = (a: string, b: string): boolean => windows ? a.toLowerCase() === b.toLowerCase() : a === b
  const base = spelled(root)
  const target = spelled(path)
  if (same(target, base))
    return ''
  const prefix = base === '/' ? '/' : `${base}/`
  return same(target.slice(0, prefix.length), prefix) ? target.slice(prefix.length) : target
}

/** Whether `path` is written as a Windows path: from a drive, `C:\`, or a share, `\\server\share`. */
function isWindowsPath(path: string): boolean {
  return /^[A-Za-z]:([\\/]|$)/.test(path) || path.startsWith('\\\\')
}

/** `path` as a person reads it on a Host whose home is `home`: `~` and `~/a/b` under it, any other path whole. */
export function homePath(path: string, home: string | null | undefined): string {
  if (!home || normalizePath(home) === '/')
    return path
  const relative = relativePath(home, path)
  if (relative === '')
    return '~'
  return relative === normalizePath(path) ? path : `~/${relative}`
}

/**
 * A directory as a list row reads it: its own name first, where it is second.
 * The home is `~` and the root `/`, each with no location; any other
 * directory is its last segment, located by its parent, which `homePath`
 * writes from `~` under the home.
 */
export function directoryParts(path: string, home: string | null | undefined): { name: string; location: string } {
  const normalized = normalizePath(path)
  if (normalized === '/')
    return { name: '/', location: '' }
  const written = homePath(normalized, home)
  if (written === '~')
    return { name: '~', location: '' }
  return { name: baseName(normalized), location: homePath(parentPath(normalized), home) }
}

/** Every ancestor from the root down to the path itself, for a breadcrumb. */
export function pathSegments(path: string): {
  name: string;
  path: string
}[] {
  const normalized = normalizePath(path)
  const crumbs = [{ name: '/', path: '/' }]
  let current = ''
  for (const segment of normalized.split('/').filter(Boolean)) {
    current += `/${segment}`
    crumbs.push({ name: segment, path: current })
  }
  return crumbs
}

export function isHiddenName(name: string): boolean {
  return name.startsWith('.')
}

/** A name a directory entry may take: no separator, not empty, not the two dot names. */
export function isValidEntryName(name: string): boolean {
  const trimmed = name.trim()
  return trimmed.length > 0 &&
    trimmed !== '.' &&
    trimmed !== '..' &&
    !trimmed.includes('/')
}

