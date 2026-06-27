// POSIX-style path helpers that also tolerate Windows separators and drive
// letters. Pure string operations — no filesystem access — so they are safe in
// any runtime. Paths are normalized to forward slashes.

/**
 * Collapse `.`/`..` segments and backslashes into a canonical forward-slash
 * path. Absolute inputs keep their leading `/` (or `DRIVE:/`); relative inputs
 * with no remaining segments become `.`.
 */
export function normalizePath(path: string): string {
  const slashPath = path.replace(/\\/g, '/')
  const drive = /^[A-Za-z]:/.exec(slashPath)?.[0].toUpperCase() ?? ''
  const absolute = slashPath.startsWith('/') || drive.length > 0
  const body = drive ? slashPath.slice(2) : slashPath
  const parts: string[] = []
  for (const segment of body.split('/')) {
    if (segment === '' || segment === '.') continue
    if (segment === '..') {
      if (parts.length > 0 && parts[parts.length - 1] !== '..') parts.pop()
      else if (!absolute) parts.push(segment)
      continue
    }
    parts.push(segment)
  }
  if (drive) return parts.length > 0 ? `${drive}/${parts.join('/')}` : `${drive}/`
  if (absolute) return `/${parts.join('/')}`
  return parts.join('/') || '.'
}

/** The parent directory of a path, after normalization (POSIX `dirname` semantics). */
export function dirnamePath(path: string): string {
  const normalized = normalizePath(path)
  if (normalized === '/' || /^[A-Za-z]:\/?$/.test(normalized)) return normalized
  const index = normalized.lastIndexOf('/')
  if (index === -1) return '.'
  if (index === 0) return '/'
  if (index === 2 && /^[A-Za-z]:/.test(normalized)) return normalized.slice(0, 3)
  return normalized.slice(0, index)
}

/** Whether a path is absolute (leading `/` or a `DRIVE:` prefix). */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path)
}
