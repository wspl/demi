import { sliceHead } from '@demicodes/utils'

/**
 * How a streaming reply's text reaches the reader (the gallery's Streaming
 * Text): a step of about `stepChars` characters at a time, cut on
 * a word, each fading in over `fadeMs`; the steps come faster as text waits,
 * so the view stays about `targetLagMs` behind what has arrived, never more
 * often than every `minStepMs` nor less than every `maxStepMs`. Many steps
 * fading at once read as the text settling a few lines at a time, as
 * Claude's own reply does, rather than as characters typed.
 */
export const STREAM_PACE = {
  stepChars: 22,
  targetLagMs: 550,
  minStepMs: 25,
  maxStepMs: 150,
  fadeMs: 400,
} as const

/** How long to wait before the next step, with `backlog` characters waiting. */
export function stepInterval(backlog: number): number {
  const ms = (STREAM_PACE.targetLagMs * STREAM_PACE.stepChars) / Math.max(1, backlog)
  return Math.min(STREAM_PACE.maxStepMs, Math.max(STREAM_PACE.minStepMs, ms))
}

const TRAILING_OPENERS = /(?:^|[\s(])(?:\*{1,3}|_{1,3}|~{1,2}|`{1,3}|\\)$/
const INCOMPLETE_LINK = /\[[^\]]*$|\[[^\]]*\]\([^)]*$/

let reducedMotionQuery: MediaQueryList | null | undefined

/** Read per frame; the query object is live, so it is created once. */
export function prefersReducedMotion(): boolean {
  if (reducedMotionQuery === undefined) {
    reducedMotionQuery = typeof matchMedia === 'function'
      ? matchMedia('(prefers-reduced-motion: reduce)')
      : null
  }
  return reducedMotionQuery?.matches ?? false
}

let wordSegmenter: Intl.Segmenter | undefined

function segmentWords(text: string): Iterable<Intl.SegmentData> {
  wordSegmenter ??= new Intl.Segmenter(undefined, { granularity: 'word' })
  return wordSegmenter.segment(text)
}

/** Word / CJK units that rejoin to the source. Spaces are their own units. */
export function segmentStreamUnits(text: string): string[] {
  if (!text)
    return []
  return Array.from(segmentWords(text), (part) => part.segment)
}

/**
 * Where the step from `from` ends in `text`: about `stepChars` characters on,
 * at a word's end (a CJK word's too, as `Intl.Segmenter` finds it), never past
 * `ceiling`, the part of `text` safe to show. A cut that would leave markdown
 * half open, which `safe` shortens, backs off to where the construct starts,
 * or takes the whole construct when it starts the step.
 */
export function nextStepEnd(text: string, from: number, ceiling: number, safe: (text: string) => string): number {
  let end = from
  for (const { segment } of segmentWords(text.slice(from, ceiling))) {
    if (end > from && end - from + segment.length > STREAM_PACE.stepChars)
      break
    end += segment.length
  }
  const cut = safe(text.slice(0, end)).length
  if (cut > from)
    return cut
  // The construct starts the step: take it whole, a word at a time, once it closes.
  for (const { segment } of segmentWords(text.slice(end, ceiling))) {
    end += segment.length
    if (safe(text.slice(0, end)).length === end)
      return end
  }
  return ceiling
}

const MARKDOWN_SYNTAX = /^ {0,3}(?:`{3,}|~{3,}).*$|\]\([^)]*\)|^ {0,3}(?:#{1,6} |> ?|[-*+] |\d+[.)] )|[*_~`[\]\n]/gm

/**
 * About how many characters `source`, a slice of markdown, renders as, line
 * breaks left out: its text without the marks around it and without a code
 * fence's lines. A step's fade covers this many rendered characters, so an
 * estimate a mark off fades a neighbouring character a moment early or late
 * and nothing more.
 */
export function renderedLength(source: string): number {
  return source.replace(MARKDOWN_SYNTAX, '').length
}

/**
 * Where, in `text`, the last `count` of its characters other than line breaks
 * start: a step's rendered length, counted as `renderedLength` counts it.
 */
export function startOfLast(text: string, count: number): number {
  let index = text.length
  for (let left = count; index > 0 && left > 0; index -= 1) {
    if (text[index - 1] !== '\n')
      left -= 1
  }
  return index
}

/** Longest prefix of `shown` that still matches `target`. */
export function alignShown(shown: string, target: string): string {
  if (target.startsWith(shown))
    return shown
  if (shown.startsWith(target))
    return target
  let index = 0
  const limit = Math.min(shown.length, target.length)
  while (index < limit && shown.charCodeAt(index) === target.charCodeAt(index)) index += 1
  if (index > 0) {
    const lead = shown.charCodeAt(index - 1)
    if (lead >= 0xd800 && lead <= 0xdbff)
      index -= 1
  }
  return target.slice(0, index)
}

/** Keep unmatched emphasis / link markers out of the markdown parse until they close. */
export function holdIncompleteMarkdown(text: string): {
  visible: string;
  held: string
} {
  if (!text)
    return { visible: '', held: '' }
  const linkAt = text.search(INCOMPLETE_LINK)
  if (linkAt >= 0)
    return {
    visible: text.slice(0, linkAt),
    held: text.slice(linkAt)
  }
  const markers = text.match(TRAILING_OPENERS)
  if (markers) {
    const token = markers[0].replace(/^[(\s]/, '')
    return { visible: text.slice(0, -token.length), held: token }
  }
  return { visible: text, held: '' }
}

const INLINE_DELIMITER = /(\*{1,3}|_{1,3}|~~|`)/g

/**
 * Closers for emphasis / strikethrough / code left open in the last paragraph, so a
 * half-written `**bold` renders bold instead of showing its asterisks until it closes.
 * Render `text + closers`; the closers add no text, so frontier tracking is unaffected.
 */
export function closeOpenInlineMarkdown(text: string): string {
  const paragraphStart = text.lastIndexOf('\n\n')
  const paragraph = paragraphStart < 0 ? text : text.slice(paragraphStart + 2)
  if (paragraph.startsWith('```') || paragraph.startsWith('~~~'))
    return ''
  const open: string[] = []
  for (const match of paragraph.matchAll(INLINE_DELIMITER)) {
    const run = match[0]
    const at = match.index
    const before = paragraph[at - 1] ?? '\n'
    const after = paragraph[at + run.length] ?? ''
    if (run === '`') {
      const top = open.at(-1)
      if (top === '`')
        open.pop()
      else open.push('`')
      continue
    }
    if (open.at(-1) === '`')
      continue
    const listMarker = (before === '\n') && /\s/.test(after) && run.length === 1
    const spaced = /\s/.test(before) && /\s/.test(after)
    if (listMarker || spaced)
      continue
    const top = open.at(-1)
    if (top && top[0] === run[0] && top.length === run.length)
      open.pop()
    else open.push(run)
  }
  return open.reverse().join('')
}
