const LINE_RANGE_SUFFIX_RE = /:\d+(?:-\d+)?$/

function joinPath(cwd: string, path: string): string {
  const trimmedCwd = cwd.endsWith('/') ? cwd.slice(0, -1) : cwd
  const trimmedPath = path.replace(/^\.\/+/, '')
  return `${trimmedCwd}/${trimmedPath}`.replace(/\/{2,}/g, '/')
}

export function normalizeFilePath(rawPath: string): string {
  const trimmedPath = rawPath.trim()
  if (!trimmedPath) return ''

  const withoutScheme = trimmedPath.startsWith('file://')
    ? trimmedPath.slice('file://'.length)
    : trimmedPath
  const withoutLineRange = withoutScheme.replace(LINE_RANGE_SUFFIX_RE, '')
  const withoutCurrentDirPrefix = withoutLineRange.replace(/^\.\/+/, '')

  if (withoutCurrentDirPrefix.length > 1 && withoutCurrentDirPrefix.endsWith('/')) {
    return withoutCurrentDirPrefix.slice(0, -1)
  }
  return withoutCurrentDirPrefix
}

export function resolveAbsolutePath(cwd: string, rawPath: string): string {
  const normalizedPath = normalizeFilePath(rawPath)
  if (!normalizedPath) return ''
  return normalizedPath.startsWith('/') ? normalizedPath : joinPath(cwd, normalizedPath)
}

export function isHttpUrl(path: string): boolean {
  return path.startsWith('http://') || path.startsWith('https://')
}

export function isLikelyFilePath(rawPath: string): boolean {
  const trimmedPath = rawPath.trim()
  const normalizedPath = normalizeFilePath(trimmedPath)
  if (!normalizedPath) return false
  if (normalizedPath.startsWith('#')) return false
  if (normalizedPath.startsWith('mailto:')) return false
  if (isHttpUrl(normalizedPath)) return false

  if (trimmedPath.endsWith('/')) return true

  return normalizedPath.startsWith('/')
    || normalizedPath.startsWith('./')
    || normalizedPath.startsWith('../')
    || normalizedPath.includes('/')
    || /^[^/\\\s]+\.[^/\\\s]+$/.test(normalizedPath)
}

export function toLocalFileUrl(filePath: string): string {
  return `local-file://${encodeURI(filePath)}`
}
