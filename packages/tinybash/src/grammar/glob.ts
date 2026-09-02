import type { HostFileSystem } from '@demicodes/shell'
import { outside } from '../outside/reasons'
import type { Piece } from './expand'

/** Whether any non-literal piece carries a pattern character. */
export function hasGlobChars(field: readonly Piece[]): boolean {
  return field.some((piece) => !piece.literal && /[*?[]/.test(piece.text))
}

/**
 * Compiles one path segment of a glob into a regex, escaping literal pieces.
 * `*` does not cross `/` (segments are matched one at a time) and a leading
 * `.` is only matched by an explicit `.`.
 */
function segmentRegex(segment: readonly Piece[]): RegExp {
  let out = '^'
  for (const piece of segment) {
    if (piece.literal) {
      out += escapeRegex(piece.text)
      continue
    }
    const text = piece.text
    for (let i = 0; i < text.length; i++) {
      const ch = text[i]!
      if (ch === '*') {
        out += '[^/]*'
      } else if (ch === '?') {
        out += '[^/]'
      } else if (ch === '[') {
        const close = findBracketClose(text, i)
        if (close === -1) {
          out += '\\['
          continue
        }
        out += bracketToRegex(text.slice(i + 1, close))
        i = close
      } else {
        out += escapeRegex(ch)
      }
    }
  }
  return new RegExp(`${out}$`)
}

function findBracketClose(text: string, open: number): number {
  let i = open + 1
  if (text[i] === '!' || text[i] === '^') i++
  if (text[i] === ']') i++
  while (i < text.length) {
    if (text[i] === '[' && text[i + 1] === ':') {
      const end = text.indexOf(':]', i + 2)
      if (end === -1) return -1
      i = end + 2
      continue
    }
    if (text[i] === ']') return i
    i++
  }
  return -1
}

const CLASSES: Record<string, string> = {
  alpha: 'A-Za-z',
  digit: '0-9',
  alnum: 'A-Za-z0-9',
  upper: 'A-Z',
  lower: 'a-z',
  space: ' \\t\\n\\r\\f\\v',
  blank: ' \\t',
  punct: '!-\\/:-@\\[-`{-~',
  xdigit: '0-9A-Fa-f',
}

/** The inside of a `[...]` bracket expression to a JS character class. */
export function bracketToRegex(inner: string): string {
  let out = '['
  let i = 0
  if (inner[0] === '!' || inner[0] === '^') {
    out += '^'
    i++
  }
  while (i < inner.length) {
    const ch = inner[i]!
    if (ch === '[' && inner[i + 1] === ':') {
      const end = inner.indexOf(':]', i + 2)
      const name = inner.slice(i + 2, end)
      const cls = CLASSES[name]
      if (cls === undefined) return '(?!)'
      out += cls
      i = end + 2
      continue
    }
    if (ch === '\\' || ch === ']' || ch === '^' || ch === '[') out += `\\${ch}`
    else out += ch
    i++
  }
  return `${out}]`
}

export function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&')
}

function firstPatternIndex(field: readonly Piece[]): number {
  let offset = 0
  for (const piece of field) {
    if (!piece.literal) {
      const index = piece.text.search(/[*?[]/)
      if (index !== -1) return offset + index
    }
    offset += piece.text.length
  }
  return -1
}

/** The directory part of a glob before its first pattern character, for the namespace check. */
export function globPrefixDir(field: readonly Piece[]): string {
  const text = field.map((piece) => piece.text).join('')
  const first = firstPatternIndex(field)
  const slash = text.lastIndexOf('/', first)
  return slash === -1 ? '' : text.slice(0, slash + 1)
}

/** Splits a field into path segments, keeping piece literalness per character. */
function splitSegments(field: readonly Piece[]): { absolute: boolean; segments: Piece[][] } {
  const segments: Piece[][] = [[]]
  let absolute = false
  let first = true
  for (const piece of field) {
    let buffer = ''
    for (const ch of piece.text) {
      if (ch === '/') {
        if (first && segments[0]!.length === 0 && buffer.length === 0) absolute = true
        if (buffer.length > 0) segments[segments.length - 1]!.push({ text: buffer, literal: piece.literal })
        buffer = ''
        segments.push([])
      } else {
        buffer += ch
      }
      first = false
    }
    if (buffer.length > 0) segments[segments.length - 1]!.push({ text: buffer, literal: piece.literal })
    first = false
  }
  return { absolute, segments: segments.filter((segment, index) => segment.length > 0 || index === segments.length - 1) }
}

function segmentHasGlob(segment: readonly Piece[]): boolean {
  return segment.some((piece) => !piece.literal && /[*?[]/.test(piece.text))
}

function segmentText(segment: readonly Piece[]): string {
  return segment.map((piece) => piece.text).join('')
}

function byteCompare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

/**
 * Pathname expansion against the filesystem, sorted; the literal word when
 * nothing matches, as bash does without `nullglob`.
 */
export async function expandGlob(field: readonly Piece[], cwd: string, fs: HostFileSystem): Promise<string[]> {
  const raw = field.map((piece) => piece.text).join('')
  if (/\*\*/.test(raw)) outside({ kind: 'grammar', found: '**', why: 'recursive globbing is not expanded here', wayOut: 'find, or list the names', line: 0 })
  const { absolute, segments } = splitSegments(field)
  const trailingSlash = raw.endsWith('/')
  const effective = trailingSlash ? segments.slice(0, -1) : segments
  let candidates: string[] = [absolute ? '/' : '']
  for (let index = 0; index < effective.length; index++) {
    const segment = effective[index]!
    const last = index === effective.length - 1
    const next: string[] = []
    for (const base of candidates) {
      if (!segmentHasGlob(segment)) {
        const name = segmentText(segment)
        const path = joinGlob(base, name)
        if (last && !trailingSlash) {
          if (await fs.exists(path, { cwd })) next.push(path)
        } else if (await isDirectory(fs, path, cwd)) {
          next.push(path)
        }
        continue
      }
      const regex = segmentRegex(segment)
      const explicitDot = segmentText(segment).startsWith('.')
      let names: string[]
      try {
        names = await fs.readdir(base === '' ? '.' : base, { cwd })
      } catch {
        continue
      }
      names.sort(byteCompare)
      for (const name of names) {
        if (name.startsWith('.') && !explicitDot) continue
        if (!regex.test(name)) continue
        const path = joinGlob(base, name)
        if (!last || trailingSlash) {
          if (!(await isDirectory(fs, path, cwd))) continue
        }
        next.push(path)
      }
    }
    candidates = next
    if (candidates.length === 0) break
  }
  if (candidates.length === 0) return [raw]
  const results = candidates.map((path) => (trailingSlash ? `${path}/` : path))
  results.sort(byteCompare)
  return results
}

function joinGlob(base: string, name: string): string {
  if (base === '') return name
  if (base.endsWith('/')) return `${base}${name}`
  return `${base}/${name}`
}

async function isDirectory(fs: HostFileSystem, path: string, cwd: string): Promise<boolean> {
  try {
    return (await fs.stat(path, { cwd })).isDirectory
  } catch {
    return false
  }
}
