import type { TinybashFs } from '../host'
import { outside } from '../outside/reasons'
import { isDirectory } from '../exec/fs'
import type { Piece } from './expand'
import { classRegexBody } from './posix-classes'
import { compareUtf8Bytes, utf8AsLatin1 } from '@demicodes/utils'

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
      out += escapeRegex(utf8AsLatin1(piece.text))
      continue
    }
    const text = utf8AsLatin1(piece.text)
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
      const cls = classRegexBody(name)
      if (cls === null) return '(?!)'
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

/**
 * Pathname expansion against the filesystem, sorted; the literal word when
 * nothing matches, as bash does without `nullglob`.
 */
export async function expandGlob(field: readonly Piece[], cwd: string, fs: TinybashFs): Promise<string[]> {
  const raw = field.map((piece) => piece.text).join('')
  if (/\*\*/.test(raw)) outside({ kind: 'grammar', found: '**', why: 'recursive globbing is not expanded here', wayOut: 'find, or list the names', line: 0 })
  const { absolute, segments } = splitSegments(field)
  const trailingSlash = raw.endsWith('/')
  const effective = trailingSlash ? segments.slice(0, -1) : segments
  // The literal segments before the first pattern are the base; nothing below them is consulted.
  const firstGlob = effective.findIndex(segmentHasGlob)
  const literal = effective.slice(0, firstGlob === -1 ? effective.length : firstGlob).map(segmentText)
  let candidates: string[] = [literal.reduce(joinGlob, absolute ? '/' : '')]
  for (let index = Math.max(firstGlob, 0); index < effective.length && firstGlob !== -1; index++) {
    const segment = effective[index]!
    const last = index === effective.length - 1
    const next: string[] = []
    for (const base of candidates) {
      if (!segmentHasGlob(segment)) {
        const name = segmentText(segment)
        const path = joinGlob(base, name)
        if (last && !trailingSlash) {
          if (await fs.exists(path, { cwd })) next.push(path)
        } else if ((await isDirectory(fs, cwd, path)) === true) {
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
      names.sort(compareUtf8Bytes)
      for (const name of names) {
        if (name.startsWith('.') && !explicitDot) continue
        if (!regex.test(utf8AsLatin1(name))) continue
        const path = joinGlob(base, name)
        if (!last || trailingSlash) {
          if ((await isDirectory(fs, cwd, path)) !== true) continue
        }
        next.push(path)
      }
    }
    candidates = next
    if (candidates.length === 0) break
  }
  if (candidates.length === 0) return [raw]
  const results = candidates.map((path) => (trailingSlash ? `${path}/` : path))
  results.sort(compareUtf8Bytes)
  return results
}

function joinGlob(base: string, name: string): string {
  if (base === '') return name
  if (base.endsWith('/')) return `${base}${name}`
  return `${base}/${name}`
}

