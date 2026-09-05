import { sliceHead } from '@demicodes/utils'

/** View catch-up for live markdown. The source string is the target; the view stays close. */
export const STREAM_REVEAL = {
  /** Comfortable writing pace. Bursts catch up instead of waiting out this rate. */
  charsPerSec: 36,
  /** Never linger more than this far behind a dumped chunk. */
  maxLagMs: 420,
  /** Finish the last units quickly when the block is no longer live. */
  flushLagMs: 180,
} as const

const TRAILING_OPENERS = /(?:^|[\s(])(?:\*{1,3}|_{1,3}|~{1,2}|`{1,3}|\\)$/
const INCOMPLETE_LINK = /\[[^\]]*$|\[[^\]]*\]\([^)]*$/

let reducedMotionQuery: MediaQueryList | null | undefined

/** Read per frame; the query object is live, so it is created once. */
export function prefersReducedMotion(): boolean {
  if (reducedMotionQuery === undefined) {
    reducedMotionQuery = typeof matchMedia === 'function' ? matchMedia('(prefers-reduced-motion: reduce)') : null
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
  if (!text) return []
  return Array.from(segmentWords(text), (part) => part.segment)
}

/** Longest prefix of `shown` that still matches `target`. */
export function alignShown(shown: string, target: string): string {
  if (target.startsWith(shown)) return shown
  if (shown.startsWith(target)) return target
  let index = 0
  const limit = Math.min(shown.length, target.length)
  while (index < limit && shown.charCodeAt(index) === target.charCodeAt(index)) index += 1
  if (index > 0) {
    const lead = shown.charCodeAt(index - 1)
    if (lead >= 0xd800 && lead <= 0xdbff) index -= 1
  }
  return target.slice(0, index)
}

// Iterates lazily: the remainder can be a whole dumped chunk while the budget is a few characters.
function takeUnits(remaining: string, budget: number): string {
  let take = ''
  for (const { segment: unit } of segmentWords(remaining)) {
    if (take.length >= budget && take.length > 0) break
    if (unit.length > budget && take.length === 0) {
      return sliceHead(unit, Math.max(budget, 1))
    }
    take += unit
  }
  return take
}

/** Next display string: word-aware. `timeLeftMs` is the deadline to finish the current gap. */
export function nextShownText(shown: string, target: string, dtMs: number, timeLeftMs: number): string {
  const aligned = alignShown(shown, target)
  if (aligned === target) return target
  const remaining = target.slice(aligned.length)
  const dt = Math.max(1, dtMs)
  const paceBudget = Math.max(1, Math.round((STREAM_REVEAL.charsPerSec * dt) / 1000))
  const catchupBudget = Math.ceil((remaining.length * dt) / Math.max(dt, timeLeftMs))
  return aligned + takeUnits(remaining, Math.max(paceBudget, catchupBudget))
}

/** Keep unmatched emphasis / link markers out of the markdown parse until they close. */
export function holdIncompleteMarkdown(text: string): { visible: string; held: string } {
  if (!text) return { visible: '', held: '' }
  const linkAt = text.search(INCOMPLETE_LINK)
  if (linkAt >= 0) return { visible: text.slice(0, linkAt), held: text.slice(linkAt) }
  const markers = text.match(TRAILING_OPENERS)
  if (markers) {
    const token = markers[0].replace(/^[(\s]/, '')
    return { visible: text.slice(0, -token.length), held: token }
  }
  return { visible: text, held: '' }
}

/** How much of `frontier` is actually at the end of the rendered visible string. */
export function visibleFrontierLength(visible: string, frontier: string): number {
  if (!frontier || !visible) return 0
  const max = Math.min(frontier.length, visible.length)
  for (let count = max; count > 0; count -= 1) {
    if (visible.endsWith(frontier.slice(frontier.length - count))) return count
  }
  return 0
}
