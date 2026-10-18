// Paths a message writes as link and image targets (`file-previews.md`
// § Files named in messages).
import { expandHome, resolveHostPath } from '../files/paths'
import type { MessageFileTarget, MessageFiles } from './types'

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

/** A commit a link names: its id or an abbreviation, and the folder of its repository when the link names one. */
export interface LinkedCommit {
  commit: string
  /** The folder as written, relative or absolute; null for the writing agent's working directory. */
  folder: string | null
}

/**
 * The commit a link's target names (`edit-tracking.md` § Commits mode):
 * `commit:3f2a9c1`, its id or at least 7 of its first hex digits, or
 * `commit:src/v8@86229da`, the repository's folder before the last `@`;
 * null for any other target.
 */
export function linkedCommit(target: string): LinkedCommit | null {
  const match = /^commit:(?:(.+)@)?([0-9a-f]{7,64})$/i.exec(target.trim())
  if (!match)
    return null
  return { commit: match[2]!, folder: match[1] === undefined ? null : decodedTarget(match[1]) }
}

/**
 * The Host folder a commit link names, resolved as a relative link's path
 * is: against the working directory, `~/` from the Host's home; null for a
 * `~/` folder while the home is unknown.
 */
export function commitFolder(folder: string, places: MessagePlaces): string | null {
  const named = places.home ? expandHome(folder, places.home) : folder
  if (named.startsWith('~/') || named === '~')
    return null
  return resolveHostPath(places.cwd, normalizeFilePath(named))
}

/** A target as written may be percent-encoded; one that does not decode stays as it is. */
export function decodedTarget(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

/** Where a message's paths start: the working directory, and the Host's home for `~/`, while it is known. */
export type MessagePlaces = Pick<MessageFiles, 'cwd' | 'home'>

/**
 * The Host path a message's link or image target names, its query and
 * fragment dropped, `~/` read from the Host's home as the File view's field
 * reads it, and a relative one resolved against the working directory; null
 * for a target that names no file, or names the home while it is unknown.
 */
export function messageHostPath(target: string, places: MessagePlaces): string | null {
  if (!isLikelyFilePath(target))
    return null
  const path = decodedTarget(normalizeFilePath(target.split(/[?#]/)[0] ?? ''))
  if (path === '')
    return null
  const named = places.home ? expandHome(path, places.home) : path
  // Without the home, a `~/` path names nothing it could be read as.
  if (named.startsWith('~/'))
    return null
  return resolveHostPath(places.cwd, named)
}

/** A line a target points at: `#L42` or `#L42-L50` after it, as GitHub links one. */
const LINE_FRAGMENT_RE = /#L([1-9]\d*)(?:-L?\d+)?$/
/** The same as a `:42` or `:42-50` after the path, as a compiler or a search names one. */
const LINE_SUFFIX_RE = /:([1-9]\d*)(?:-\d+)?$/

/**
 * The line a message's link target points at, its first when it names a
 * range (`file-previews.md` § Files named in messages); null when it points
 * at none.
 */
export function targetLine(target: string): number | null {
  const trimmed = target.trim()
  const line = LINE_FRAGMENT_RE.exec(trimmed)?.[1]
    ?? LINE_SUFFIX_RE.exec(trimmed.split(/[?#]/)[0] ?? '')?.[1]
  return line === undefined ? null : Number(line)
}

/** The Host file a link's target names, resolved from `places`, with its line; null for a target that names none. */
export function messageFileTarget(target: string, places: MessagePlaces): MessageFileTarget | null {
  const path = messageHostPath(target, places)
  return path === null ? null : { path, line: targetLine(target) }
}

/**
 * How a link to the web opens in a new tab: with no hold on the page, no
 * referrer, which on a shared page would carry its link, and no
 * endorsement a crawler would follow (`sharing.md` § Safety).
 */
export const WEB_LINK_REL = 'noopener noreferrer nofollow'

export function isHttpUrl(path: string): boolean {
  return path.startsWith('http://') || path.startsWith('https://')
}

/** Whether a target names a file rather than a web page, a fragment, another scheme or a bare word. */
export function isLikelyFilePath(rawPath: string): boolean {
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

/**
 * Where an image a message names loads from: a web image or a `data:` URL
 * from its `url`, a Host image from the file at `path`, in the version the
 * page keeps of it (`useHostImagesAt`). Null for a target that loads nothing,
 * which shows its alt text. A click on it shows it large in the media viewer.
 */
export function messageImage(target: string, files: MessageFiles | undefined): { url: string } | { path: string } | null {
  if (isHttpUrl(target) || target.startsWith('data:'))
    return { url: target }
  if (!files)
    return null
  const path = messageHostPath(target, files)
  return path === null ? null : { path }
}
