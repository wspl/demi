// Paths a message writes as link and image targets (`file-previews.md`
// § Files named in messages).
import { resolveHostPath } from '../files/paths'

const LINE_RANGE_SUFFIX_RE = /:\d+(?:-\d+)?$/

/** A URL scheme of two or more letters; a single letter before `:` is a drive. */
const SCHEME_RE = /^[a-z][a-z0-9+.-]+:/i

/**
 * A target as the path it names: without a `file://` scheme, a `:line`
 * suffix, a leading `./` or a trailing slash.
 */
function normalizeFilePath(rawPath: string): string {
  const trimmedPath = rawPath.trim()
  if (!trimmedPath)
    return ''

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

/** A target as written may be percent-encoded; one that does not decode stays as it is. */
export function decodedTarget(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

/**
 * The Host path a message's link or image target names, its query and
 * fragment dropped and resolved against the working directory; null for a
 * target that names no file.
 */
export function messageHostPath(target: string, cwd: string): string | null {
  if (!isLikelyFilePath(target))
    return null
  const path = decodedTarget(normalizeFilePath(target.split(/[?#]/)[0] ?? ''))
  return path === '' ? null : resolveHostPath(cwd, path)
}

export function isHttpUrl(path: string): boolean {
  return path.startsWith('http://') || path.startsWith('https://')
}

/** Whether a target names a file rather than a web page, a fragment, another scheme or a bare word. */
function isLikelyFilePath(rawPath: string): boolean {
  const trimmedPath = rawPath.trim()
  const normalizedPath = normalizeFilePath(trimmedPath)
  if (!normalizedPath)
    return false
  if (normalizedPath.startsWith('#'))
    return false
  if (SCHEME_RE.test(normalizedPath))
    return false

  // A directory, a path with a separator, or a file name with an extension.
  return trimmedPath.endsWith('/')
    || /[/\\]/.test(normalizedPath)
    || /^[^/\\\s]+\.[^/\\\s]+$/.test(normalizedPath)
}
