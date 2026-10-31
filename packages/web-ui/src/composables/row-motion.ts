import { CHROME_ROLL_EASING } from '../ui/chrome-roll'
import { prefersReducedMotion } from '../ui/stream-reveal'

/** How long the rows of a list take to move to their places when content comes or goes. */
export const ROW_MOVE_MS = 200

/** The name of a row's move, which a later move replaces. */
const ROW_MOVE_ID = 'row-move'
/** What a row element is found by, and named by, in the list. */
const ROW = '[data-row-key]'

/** The rendered message in a row, whose last block is where the row's text ends. */
const MESSAGE = '.markdown-body'

/**
 * The last block of a row's text, by its place among its message's blocks (a
 * message is drawn anew whenever its text changes, so the element itself is
 * never the same), and how far below the row's top it stood.
 */
export interface RowTail {
  index: number
  offset: number
}

/** The last block of the row's last message. */
function lastBlock(row: HTMLElement): { message: Element, block: Element } | null {
  const messages = row.querySelectorAll(MESSAGE)
  const message = messages[messages.length - 1]
  const block = message?.lastElementChild
  return message && block ? { message, block } : null
}

/** The tail of every row, by its key, as the rows stand now. */
export function rowTails(scroller: HTMLElement): Map<string, RowTail> {
  const tails = new Map<string, RowTail>()
  for (const row of scroller.querySelectorAll<HTMLElement>(ROW)) {
    const last = lastBlock(row)
    if (last)
      tails.set(row.dataset['rowKey']!, { index: last.message.childElementCount - 1, offset: last.block.getBoundingClientRect().top - row.getBoundingClientRect().top })
  }
  return tails
}

/** How far a row's tail block stands lower in it than it did, as when something above it grew. */
export function tailDisplaced(row: HTMLElement, was: RowTail | undefined): number {
  const message = was && row.querySelectorAll(MESSAGE)
  const block = message?.[message.length - 1]?.children[was!.index]
  return was && block ? block.getBoundingClientRect().top - row.getBoundingClientRect().top - was.offset : 0
}

/**
 * Where each row stood on the screen and how tall it was, by its key, how far
 * the scroller was scrolled, and where each row's text ended in it when the
 * list last settled.
 */
export interface RowsBefore {
  tops: Map<string, number>
  heights: Map<string, number>
  tails: Map<string, RowTail>
  scrollTop: number
}

/**
 * Where the rows are drawn now, as the reader sees them: their tops relative to
 * `scroller`'s, a move still running included. `heights` is how tall the list
 * placed the rows when it last settled.
 */
export function rowsBefore(scroller: HTMLElement, heights: Map<string, number>, tails: Map<string, RowTail>): RowsBefore {
  const top = scroller.getBoundingClientRect().top
  const tops = new Map<string, number>()
  for (const row of scroller.querySelectorAll<HTMLElement>(ROW))
    tops.set(row.dataset['rowKey']!, row.getBoundingClientRect().top - top)
  return { tops, heights, tails, scrollTop: scroller.scrollTop }
}

/**
 * `before` as the reader sees it after they scrolled the list to `scrollTop`:
 * every row stands that much further along, so what they scrolled is no move of
 * any row.
 */
export function scrolledRows(before: RowsBefore, scrollTop: number): RowsBefore {
  const by = scrollTop - before.scrollTop
  return { tops: new Map([...before.tops].map(([key, top]) => [key, top - by])), heights: before.heights, tails: before.tails, scrollTop }
}

/** The moves of a row that are running. */
function moving(row: HTMLElement): Animation[] {
  return row.getAnimations().filter((move) => move.id === ROW_MOVE_ID)
}

/** Puts every row of `scroller` at its place at once, ending the moves that run. */
export function stopRowMoves(scroller: HTMLElement): void {
  for (const row of scroller.querySelectorAll<HTMLElement>(ROW))
    for (const move of moving(row))
      move.cancel()
}

/** The properties whose animation changes an element's height. */
const HEIGHT_PROPERTIES = new Set(['height', 'maxHeight', 'blockSize', 'gridTemplateRows', 'grid-template-rows', 'max-height', 'block-size'])

/** Whether `animation` changes the height of its element, as a fold or a roll of a row's face does. */
function changesHeight(animation: Animation): boolean {
  if (animation.id === ROW_MOVE_ID)
    return false
  if (animation instanceof CSSTransition)
    return HEIGHT_PROPERTIES.has(animation.transitionProperty)
  const effect = animation.effect
  return effect instanceof KeyframeEffect && effect.getKeyframes().some((frame) => Object.keys(frame).some((name) => HEIGHT_PROPERTIES.has(name)))
}

/**
 * Moves each row to the place it now has from where it stood (`before`), over
 * `ROW_MOVE_MS`, so content that came or went never moves anything in one frame:
 * the rows above a row added at the end of a list that follows it slide up as
 * the list scrolls, and the rows below a row that goes slide up to its place.
 *
 * A row the list did not draw before comes in from where the scroll carried
 * the end: from below, when the list `following` its end scrolled to follow,
 * and not at all otherwise, as when it came by the reader's scroll or by a
 * correction of the rows' sizes. A move longer than the view is not followed
 * by anyone and does not move.
 *
 * Rows never overlap. A row that grew or shrank by itself, as a stream's text
 * does, takes the rows below it with it in the frame it changes, as if they had
 * stood there before, so they move only as the list scrolls to follow its end,
 * with every row. A row that comes in starts no higher than the row above it
 * does.
 *
 * A row that grew above its last block, as a formula does when it is drawn,
 * keeps that block where it stood: the list scrolls to follow its end by as
 * much, and the rows above are not drawn back to where they stood but go with
 * the scroll. The reader reads the end of the text and sees it still, never
 * pushed down and drawn back; what lies above the growth moves once, as the
 * page does above an image that has loaded.
 *
 * A row whose own height is animating, as a fold or the roll of a face is,
 * gets no move of its own, and the rows below it follow it frame by frame: a
 * move over that would lag behind the row and draw them under it. So does the
 * list's follow of a growing row, which moves the rows above it in each frame
 * too: they slide only when rows came or went.
 *
 * The rows' own transform places them (`translateY` to their start), so each
 * move is added to it, and a move still running is replaced by one from where
 * its row stood, while one that is not replaced goes on.
 */
export function moveRows(scroller: HTMLElement, before: RowsBefore, heights: Map<string, number>, following: boolean): void {
  const rows = [...scroller.querySelectorAll<HTMLElement>(ROW)]
  if (typeof rows[0]?.animate !== 'function')
    return
  if (prefersReducedMotion() || document.hidden) {
    stopRowMoves(scroller)
    return
  }
  const keys = rows.map((row) => row.dataset['rowKey']!)
  const growing = rows.findIndex((row) => row.getAnimations({ subtree: true }).some(changesHeight))
  const changed = keys.some((key) => !before.tops.has(key)) || [...before.tops.keys()].some((key) => !keys.includes(key))
  const starts = rows.flatMap((row, index) =>
    growing < 0 || !before.tops.has(keys[index]!) || (changed && index < growing) ? [row] : [])
  for (const row of starts)
    for (const move of moving(row))
      move.cancel()
  const top = scroller.getBoundingClientRect().top
  const scrolled = scroller.scrollTop - before.scrollTop
  const view = scroller.clientHeight
  // How far the rows above a row grew or shrank by themselves, in rows order.
  const grown = new Map<string, number>()
  let by = 0
  for (const row of rows) {
    const key = row.dataset['rowKey']!
    grown.set(key, by)
    const was = before.heights.get(key)
    const now = heights.get(key)
    if (was !== undefined && now !== undefined)
      by += now - was
  }
  // The first row that grew above its last block, and by how much.
  let tailRow = -1
  let tailBy = 0
  for (const [index, row] of rows.entries()) {
    const key = row.dataset['rowKey']!
    const by = before.tops.has(key) ? tailDisplaced(row, before.tails.get(key)) : 0
    if (by >= 1) {
      tailRow = index
      tailBy = by
      break
    }
  }
  // Where the row above starts, relative to its place, and where it ends.
  let aboveOffset = 0
  let aboveBottom = Number.NEGATIVE_INFINITY
  for (const [index, row] of rows.entries()) {
    const key = row.dataset['rowKey']!
    const box = row.getBoundingClientRect()
    const now = box.top - top
    const was = before.tops.get(key)
    let from = 0
    if (starts.includes(row)) {
      if (was !== undefined) {
        from = was + grown.get(key)! - now
        if (from > 0 && index <= tailRow)
          from = Math.max(0, from - tailBy)
      }
      else if (following && scrolled > 0)
        from = scrolled
      // A row that comes in never starts over the row above it.
      if (was === undefined)
        from = Math.max(from, aboveOffset - (now - aboveBottom))
    }
    aboveBottom = now + box.height
    if (Math.abs(from) < 1 || Math.abs(from) > view) {
      aboveOffset = 0
      continue
    }
    aboveOffset = from
    const move = row.animate(
      [{ transform: `translateY(${from}px)` }, { transform: 'translateY(0px)' }],
      { duration: ROW_MOVE_MS, easing: CHROME_ROLL_EASING, composite: 'add' },
    )
    move.id = ROW_MOVE_ID
    // A move that a later one replaces is canceled, which rejects `finished`.
    move.finished.catch(() => {})
  }
}
