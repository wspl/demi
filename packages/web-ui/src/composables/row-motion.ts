import { CHROME_ROLL_EASING } from '../ui/chrome-roll'
import { prefersReducedMotion } from '../ui/stream-reveal'

/** How long the rows of a list take to move to their places when content comes or goes. */
export const ROW_MOVE_MS = 200

/** The name of a row's move, which a later move replaces. */
const ROW_MOVE_ID = 'row-move'
/** What a row element is found by, and named by, in the list. */
const ROW = '[data-row-key]'

/** Where each row stood on the screen and how tall it was, by its key, and how far the scroller was scrolled. */
export interface RowsBefore {
  tops: Map<string, number>
  heights: Map<string, number>
  scrollTop: number
}

/**
 * Where the rows are drawn now, as the reader sees them: their tops relative to
 * `scroller`'s, a move still running included. `heights` is how tall the list
 * placed the rows when it last settled.
 */
export function rowsBefore(scroller: HTMLElement, heights: Map<string, number>): RowsBefore {
  const top = scroller.getBoundingClientRect().top
  const tops = new Map<string, number>()
  for (const row of scroller.querySelectorAll<HTMLElement>(ROW))
    tops.set(row.dataset['rowKey']!, row.getBoundingClientRect().top - top)
  return { tops, heights, scrollTop: scroller.scrollTop }
}

/**
 * `before` as the reader sees it after they scrolled the list to `scrollTop`:
 * every row stands that much further along, so what they scrolled is no move of
 * any row.
 */
export function scrolledRows(before: RowsBefore, scrollTop: number): RowsBefore {
  const by = scrollTop - before.scrollTop
  return { tops: new Map([...before.tops].map(([key, top]) => [key, top - by])), heights: before.heights, scrollTop }
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
  const moving = (row: HTMLElement) => row.getAnimations().filter((move) => move.id === ROW_MOVE_ID)
  if (prefersReducedMotion() || document.hidden) {
    for (const row of rows)
      for (const move of moving(row))
        move.cancel()
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
  // Where the row above starts, relative to its place, and where it ends.
  let aboveOffset = 0
  let aboveBottom = Number.NEGATIVE_INFINITY
  for (const row of rows) {
    const key = row.dataset['rowKey']!
    const box = row.getBoundingClientRect()
    const now = box.top - top
    const was = before.tops.get(key)
    let from = 0
    if (starts.includes(row)) {
      if (was !== undefined)
        from = was + grown.get(key)! - now
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
