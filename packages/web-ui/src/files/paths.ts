/** POSIX path arithmetic for the file browser: pure, no filesystem. */

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

/** The root is its own parent. */
export function parentPath(path: string): string {
  const normalized = normalizePath(path)
  const index = normalized.lastIndexOf('/')
  return index <= 0 ? '/' : normalized.slice(0, index)
}

/** The last segment; the root has the empty name. */
export function baseName(path: string): string {
  const normalized = normalizePath(path)
  return normalized.slice(normalized.lastIndexOf('/') + 1)
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

/** `Home` shortens the home directory to `~`; other paths stay absolute. */
export function displayPath(path: string, home?: string): string {
  if (!home)
    return path
  const normalizedHome = normalizePath(home)
  if (path === normalizedHome)
    return '~'
  if (path.startsWith(`${normalizedHome}/`))
    return `~${path.slice(normalizedHome.length)}`
  return path
}
