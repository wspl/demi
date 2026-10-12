import { sliceHead } from '@demicodes/utils'
import type { TextArrival } from '@demicodes/conversation-client'

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

/** A part of the text the reader was shown at once, `start` to `end` in the source, at `at` (performance.now()). */
export interface RevealStep {
  start: number
  end: number
  at: number
}

/**
 * The steps of a text shown whole up to `shown` at `now`, a step for each
 * piece that arrived (`arrivals`) less than a fade ago, fading from its
 * arrival: what a view that mounts on a streaming text shows still fading.
 */
export function arrivedSteps(arrivals: readonly TextArrival[], shown: number, now: number): RevealStep[] {
  const steps: RevealStep[] = []
  let start = 0
  for (const { end, at } of arrivals) {
    if (now - at < STREAM_PACE.fadeMs && start < shown && end > start)
      steps.push({ start, end: Math.min(end, shown), at })
    start = end
  }
  return steps
}

/** How a step's text fades in. */
const FADE_IN: Keyframe[] = [{ opacity: 0 }, { opacity: 1 }]

/**
 * Gives the text of each step still fading in a span that fades it, from the
 * render's end backwards. A step's extent is its rendered length, counted
 * from the end; code keeps its place in the count but no span, as a code
 * block's text shows at once.
 *
 * Each fade is an animation of the document's timeline that starts when its
 * step was shown (`performance.now()` and the timeline count from the same
 * origin), so it goes on unbroken through a render, which replaces the spans,
 * and through a move of the text in the page, which a CSS animation would
 * start over: setting up a scroller's bars (`ScrollArea`) appends its
 * content to it again, and the reply's last lines faded in a second time.
 *
 * The timeline reads the time its frame began, which can be earlier than the
 * `performance.now()` a step was shown at, so the first frame that draws a
 * step may come before its fade starts. Until then the text keeps the fade's
 * first keyframe (`backwards`); without it, that frame painted the text
 * solid, the next one nearly transparent, and it faded in from there.
 */
export function wrapSteps(el: HTMLElement, text: string, shownSteps: readonly RevealStep[]): void {
  if (prefersReducedMotion())
    return
  const now = performance.now()
  const marks = shownSteps
    .filter((step) => now - step.at < STREAM_PACE.fadeMs && step.start < text.length)
    .toSorted((a, b) => b.start - a.start)
    .map((step) => ({ fromEnd: renderedLength(text.slice(step.start)), at: step.at }))
  if (marks.length === 0)
    return
  let counted = 0
  let mark = 0
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  for (
    let node = walker.lastChild() as Text | null;
    node && mark < marks.length;
    node = walker.previousNode() as Text | null
  ) {
    const content = node.textContent ?? ''
    // Whitespace between blocks (the newline marked emits after each paragraph) carries no
    // ink, and the count leaves out the newlines between blocks.
    if (!content.trim())
      continue
    const code = node.parentElement?.closest('pre') !== null
    // `end` and `left` walk the node's text from its end; line breaks take no part in the count.
    let end = content.length
    let left = content.length - (content.match(/\n/g)?.length ?? 0)
    while (left > 0 && mark < marks.length) {
      const { fromEnd, at } = marks[mark]!
      const take = Math.min(fromEnd - counted, left)
      const start = startOfLast(content.slice(0, end), take)
      if (take > 0 && !code) {
        // A span of inline code shown whole in one step fades with its fill, never as an empty box first.
        const parent = node.parentElement
        const inked = start === 0 && end === content.length && parent?.tagName === 'CODE' && parent.childNodes.length === 1
          ? parent
          : document.createElement('span')
        if (inked !== parent) {
          const piece = node.splitText(start)
          piece.replaceWith(inked)
          inked.append(piece)
        }
        // The class marks the text still fading in, for whoever looks at the page.
        inked.classList.add('stream-ink')
        inked.animate(FADE_IN, { duration: STREAM_PACE.fadeMs, fill: 'backwards' }).startTime = at
      }
      end = start
      left -= take
      counted += take
      if (counted >= fromEnd)
        mark += 1
    }
  }
}
