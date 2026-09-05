/**
 * The POSIX character classes of the C locale as byte ranges: `[:alpha:]`
 * and friends in glob brackets, grep bracket expressions and tr sets all
 * read this one table.
 */
const RANGES: Record<string, readonly (readonly [number, number])[]> = {
  upper: [[65, 90]],
  lower: [[97, 122]],
  digit: [[48, 57]],
  alpha: [[65, 90], [97, 122]],
  alnum: [[48, 57], [65, 90], [97, 122]],
  space: [[9, 13], [32, 32]],
  blank: [[9, 9], [32, 32]],
  punct: [[33, 47], [58, 64], [91, 96], [123, 126]],
  print: [[32, 126]],
  graph: [[33, 126]],
  cntrl: [[0, 31], [127, 127]],
  xdigit: [[48, 57], [65, 70], [97, 102]],
}

/** The bytes of a class, or null for a name POSIX does not define. */
export function classBytes(name: string): number[] | null {
  const ranges = RANGES[name]
  if (ranges === undefined) return null
  const out: number[] = []
  for (const [lo, hi] of ranges) for (let c = lo; c <= hi; c++) out.push(c)
  return out
}

const hex = (c: number) => `\\x${c.toString(16).padStart(2, '0')}`

/** The class as the body of a JS character class (`A-Za-z`), or null for an unknown name. */
export function classRegexBody(name: string): string | null {
  const ranges = RANGES[name]
  if (ranges === undefined) return null
  return ranges.map(([lo, hi]) => (lo === hi ? hex(lo) : `${hex(lo)}-${hex(hi)}`)).join('')
}
