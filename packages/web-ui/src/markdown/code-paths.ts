// Inline code that names a file of the Host, as `docs/execution/runner.md`
// or `src/app.ts:42`: the page asks the conversation's Host which of a reply's
// are files, and those open as links.
import { resolveHostPath } from '../files/paths'
import type { MessageFileTarget } from './types'

/** The most paths one reply asks about: the backend's bound on one question. */
export const CODE_PATHS_MAX = 200

/** A URL scheme of two or more letters; a single letter before `:` is a drive. */
const SCHEME_RE = /^[a-z][a-z0-9+.-]+:/i
/** Characters a shell or a pattern gives meaning to, which a path written as code does not hold. */
const NOT_IN_PATH_RE = /[\s*?<>|"'`$;,\\]/
/** A name with an extension: some of a name, a dot, and a suffix that starts with a letter. */
const NAME_WITH_EXTENSION_RE = /^[^./][^/]*\.[A-Za-z][A-Za-z0-9_-]*$/
/** A drive's start, as `C:/`. */
const DRIVE_RE = /^[A-Za-z]:\//

/**
 * The path and line an inline code span names, as the agent wrote it: one
 * with a `/` or a name with an extension, with an optional `:line`, and no
 * white space; null for code that reads as no path, such as a bare name
 * (`runner`), a URL, a command or a directory (`src/`). A bare name with an
 * extension, such as `README.md`, names the file in the working directory.
 */
export function codePath(code: string): { path: string; line: number | null } | null {
  const suffix = /:([1-9]\d*)$/.exec(code)
  const path = suffix ? code.slice(0, suffix.index) : code
  if (path === '' || NOT_IN_PATH_RE.test(path) || SCHEME_RE.test(path) || path.startsWith('-') || path.startsWith('~'))
    return null
  if (path.includes(':') && !(DRIVE_RE.test(path) && !path.slice(2).includes(':')))
    return null
  if (path.endsWith('/') || path.endsWith('.'))
    return null
  const named = path.includes('/') ? path.split('/').some((part) => part !== '' && part !== '.' && part !== '..') : NAME_WITH_EXTENSION_RE.test(path)
  return named ? { path, line: suffix ? Number(suffix[1]) : null } : null
}

/** The Host file inline code names, resolved against `cwd`, with its line; null for code that names none. */
export function codeFileTarget(code: string, cwd: string): MessageFileTarget | null {
  const named = codePath(code)
  return named ? { path: resolveHostPath(cwd, named.path), line: named.line } : null
}

/**
 * The Host paths a message's inline code spans name, resolved against
 * `cwd`, each once, in the order the message first names them, and no more
 * than one question asks about.
 */
export function messageCodePaths(spans: readonly string[], cwd: string): string[] {
  const paths = new Set<string>()
  for (const code of spans) {
    const target = codeFileTarget(code, cwd)
    if (target)
      paths.add(target.path)
  }
  return [...paths].slice(0, CODE_PATHS_MAX)
}
