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
// A link's text, closed or not, with nothing after it yet, or with its target half written.
const INCOMPLETE_LINK = /\[[^\]]*\]?$|\[[^\]]*\]\([^)]*$/
/**
 * A last line that opens a block and has no text yet: a list item's or a
 * quote's markers, or a code fence. Shown alone it would draw the block's
 * marker, bar or well before any of its text.
 */
const EMPTY_BLOCK_OPENER = /(?<=^|\n)(?:[ \t]*(?:[-*+]|\d{1,9}[.)]|>)(?:[ \t]+|$))+$|(?<=^|\n) {0,3}(?:`{3,}|~{3,})[^`\n]*\n?$/

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

const MARKDOWN_SYNTAX = /^ {0,3}(?:`{3,}|~{3,}).*$|^ {0,3}(?:-{3,}|\|?(?: *:?-+:? *\|)+(?: *:?-+:? *)?) *$|^ {0,3}\| *|(?<=^ {0,3}\|.*?) *\| *|\]\([^)]*\)|^ {0,3}(?:#{1,6} |> ?|[-*+] |\d+[.)] )|[*_~`[\]\n]/gm

/**
 * About how many characters `source`, a slice of markdown, renders as, line
 * breaks left out: its text without the marks around it, without a code
 * fence's lines, a rule's dashes, and a table's pipes and the row that aligns
 * its columns. A step's fade covers this many rendered characters, so an
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

/**
 * Keep unmatched emphasis / link markers out of the markdown parse until they
 * close, and a block's opening line out until its first text.
 */
export function holdIncompleteMarkdown(text: string): {
  visible: string;
  held: string
} {
  if (!text)
    return { visible: '', held: '' }
  const openerAt = text.search(EMPTY_BLOCK_OPENER)
  if (openerAt >= 0)
    return { visible: text.slice(0, openerAt), held: text.slice(openerAt) }
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
 * The elements of a render that paint something of their own around their
 * text: inline code's box, a link's underline and icon, a key's frame, a list
 * item's marker, a quote's bar, a code block's well and language, a table's
 * frame and a row's rules and fill.
 */
const OWN_PAINT = 'code, a, kbd, .icon-link, li, blockquote, .code-block, pre, table, tr'

/**
 * Whether `node` is text of the reply, as a step's rendered length counts
 * it: not the whitespace between blocks (the newline marked emits after each
 * paragraph), nor a code block's language, which the fence line names.
 */
function isReplyText(node: Text): boolean {
  return (node.textContent ?? '').trim() !== '' && !node.parentElement?.closest('.code-block-lang')
}

/** A part of a text node a step showed: `start` to `end` in it, of the step at `mark`. */
interface Piece {
  node: Text
  start: number
  end: number
  mark: number
}

/**
 * Fades in the text of each step still fading, from the render's end
 * backwards. A step's extent is its rendered length, counted from the end.
 *
 * An element that paints something of its own (`OWN_PAINT`) fades with the
 * step that shows its first character, with the text of that step inside it;
 * its text of later steps fades again inside it, from where the element's
 * fade has got to. So inline code's box comes with its first characters,
 * never as an empty box first, and a list's marker with its item's first
 * word. Other text of a step fades in a span of its own; a code block's
 * text shows with its block, as code is not shown a word at a time.
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
  const fadingSteps = shownSteps.filter((step) => now - step.at < STREAM_PACE.fadeMs && step.start < text.length)
  if (fadingSteps.length === 0)
    return
  // Counted as the whole text less what comes before the step, so a step that
  // starts within a line, such as a table's row, is read in that line's context.
  const total = renderedLength(text)
  const marks = fadingSteps
    .toSorted((a, b) => b.start - a.start)
    .map((step) => ({ fromEnd: total - renderedLength(text.slice(0, step.start)), at: step.at }))

  // Which step shows each piece of text, read before the render changes.
  const pieces: Piece[] = []
  /** The step of each text node's first character, for the nodes a fading step starts in. */
  const firstMarks = new Map<Text, number>()
  let counted = 0
  let mark = 0
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  for (
    let node = walker.lastChild() as Text | null;
    node && mark < marks.length;
    node = walker.previousNode() as Text | null
  ) {
    if (!isReplyText(node))
      continue
    const content = node.textContent ?? ''
    // `end` and `left` walk the node's text from its end; line breaks take no part in the count.
    let end = content.length
    let left = content.length - (content.match(/\n/g)?.length ?? 0)
    while (left > 0 && mark < marks.length) {
      const take = Math.min(marks[mark]!.fromEnd - counted, left)
      const start = startOfLast(content.slice(0, end), take)
      if (take > 0) {
        pieces.push({ node, start, end, mark })
        // The piece that takes the rest of the node holds its first character.
        if (take === left)
          firstMarks.set(node, mark)
      }
      end = start
      left -= take
      counted += take
      if (counted >= marks[mark]!.fromEnd)
        mark += 1
    }
  }

  // The painting elements whose first character a fading step shows, each
  // fading with that step unless an element around it already does.
  const fading = new Map<Element, number>()
  for (const element of el.querySelectorAll(OWN_PAINT)) {
    const first = firstReplyText(element)
    const step = first ? firstMarks.get(first) : undefined
    if (step === undefined)
      continue
    if (fadingAround(element, el, fading)?.[1] === step)
      continue
    fading.set(element, step)
    fade(element, marks[step]!.at)
  }

  for (const { node, start, end, mark: step } of pieces) {
    const around = fadingAround(node, el, fading)
    // The text of an element's own fade fades with it, never twice; code shows with its block.
    if (around?.[1] === step || node.parentElement?.closest('pre'))
      continue
    // Splitting from the end leaves the node's earlier pieces where they are.
    const piece = start > 0 ? node.splitText(start) : node
    if (end < (piece.textContent ?? '').length)
      piece.splitText(end - start)
    const span = document.createElement('span')
    piece.replaceWith(span)
    span.append(piece)
    fade(span, marks[step]!.at)
  }
}

/** The first text of the reply inside `element`. */
function firstReplyText(element: Element): Text | null {
  const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
    if (isReplyText(node))
      return node
  }
  return null
}

/** The nearest element around `node`, inside `root`, that fades, with its step. */
function fadingAround(node: Node, root: Element, fading: ReadonlyMap<Element, number>): [Element, number] | undefined {
  for (let element = node.parentElement; element && element !== root; element = element.parentElement) {
    const step = fading.get(element)
    if (step !== undefined)
      return [element, step]
  }
  return undefined
}

/** Fades `element` in from `at`, the time its step was shown. */
function fade(element: Element, at: number): void {
  // The class marks what is still fading in, for whoever looks at the page.
  element.classList.add('stream-ink')
  element.animate(FADE_IN, { duration: STREAM_PACE.fadeMs, fill: 'backwards' }).startTime = at
}
