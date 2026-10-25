import { shallowRef } from 'vue'
import { isPicture, PICTURE } from './context-menu'

/**
 * A selection a list draws itself in place of the browser's, so that it
 * looks the same while the page has the focus and while a field that acts
 * on it does (`DrawnSelection.vue`):
 *
 * ```text
 *   Uploads fail on slow     ╭───────────────────╮
 *   connections              │because the client │  a strip per line, 4px
 *                            ╰───────────────────╯  apart, from its first
 *   ╭────────────────────────────────────╮          selected letter to its
 *   │request runs under the same timeout │          last, and a space more
 *   ╰────────────────────────────────────╯          where the selection
 *                                                   goes on past the line
 *    ┏━━━━━━━━━━┓
 *    ┃ picture  ┃  a picture or a file card: an outline
 *    ┗━━━━━━━━━━┛
 * ```
 *
 * A line's pieces, as inline code, a link or bold words, merge into one
 * strip; the lines of two blocks, as two paragraphs or two table cells, do
 * not. A line the selection goes on past ends a space's width after its last
 * letter, where the browser draws the line's end as selected. Only the painting is the list's: the selection is still the
 * browser's, which copies, drags and opens its menu.
 */

/**
 * A range the page draws as selected though the browser's selection is not
 * on it: the picture a menu opened a comment field on, which the browser does
 * not select. Null when there is none.
 */
export const heldSelection = shallowRef<Range | null>(null)

/** A box in the list's own coordinates. */
export interface DrawnBox {
  top: number
  left: number
  width: number
  height: number
}

/**
 * A picture's box, the radius of its corners, and how much of each side the
 * box it scrolls in hides, as `clip-path: inset()` takes it: negative where
 * the outline around it shows.
 */
export interface DrawnPicture extends DrawnBox {
  radius: number
  clip: string
}

export interface DrawnShapes {
  strips: DrawnBox[]
  pictures: DrawnPicture[]
}

export const NO_SHAPES: DrawnShapes = { strips: [], pictures: [] }

/** The space between the strips of two lines. */
const GAP = 4
/** How far a picture's outline reaches out of its box: `.drawn-selection-picture`'s offset and width. */
const OUTLINE = 4
/** A file card, which a selection outlines as it does a picture, though its own words are not selectable. */
const CARD = '[data-attachment-card-body]'

interface Line {
  block: Element
  mid: number
  height: number
  left: number
  right: number
  /** The font of the line's last letter, which the width of the space at its end is measured in. */
  font: string
}

/**
 * The strips and outlines that draw what `range` selects in `root`, relative
 * to `root`'s box. What it selects outside `root` is not drawn.
 */
export function selectionShapes(range: Range, root: HTMLElement): DrawnShapes {
  if (!range.intersectsNode(root))
    return NO_SHAPES
  const styles = new Map<Element, CSSStyleDeclaration>()
  const style = (element: Element) => {
    let found = styles.get(element)
    if (!found) {
      found = getComputedStyle(element)
      styles.set(element, found)
    }
    return found
  }
  const unselectable = new Map<Element, boolean>()
  /** Whether the browser selects no text in `element`: its `user-select`, or one around it in `root`, is none. */
  const selectsNone = (element: Element | null): boolean => {
    if (!element || element === root || !root.contains(element))
      return false
    let known = unselectable.get(element)
    if (known === undefined) {
      const own = style(element)
      known = (own.userSelect || own.webkitUserSelect) === 'none' || selectsNone(element.parentElement)
      unselectable.set(element, known)
    }
    return known
  }
  /** The block a line of `element`'s text belongs to: the nearest element around it that is no inline box. */
  const blockOf = (element: Element): Element => {
    let at = element
    while (at !== root && at.parentElement && /^(inline|contents)/.test(style(at).display))
      at = at.parentElement
    return at
  }

  const lines: Line[] = []
  const pictures: Element[] = []
  const take = (node: Node) => {
    if (node instanceof Element) {
      if (isPicture(node) || node.matches(CARD))
        pictures.push(node)
      return
    }
    const parent = node.parentElement
    if (!(node instanceof Text) || !parent || parent.closest(`${PICTURE}, ${CARD}`) || selectsNone(parent))
      return
    const part = document.createRange()
    part.setStart(node, node === range.startContainer ? range.startOffset : 0)
    part.setEnd(node, node === range.endContainer ? range.endOffset : node.length)
    const block = blockOf(parent)
    const blockStyle = style(block)
    const height = Number.parseFloat(blockStyle.lineHeight) || Number.parseFloat(blockStyle.fontSize) * 1.2
    const font = style(parent).font
    for (const rect of part.getClientRects()) {
      if (rect.width < 1)
        continue
      const mid = rect.top + rect.height / 2
      const line = lines.find((other) => other.block === block && Math.abs(other.mid - mid) < height / 2)
      if (line) {
        line.left = Math.min(line.left, rect.left)
        if (rect.right >= line.right) {
          line.right = rect.right
          line.font = font
        }
      } else {
        lines.push({ block, mid, height, left: rect.left, right: rect.right, font })
      }
    }
  }

  const around = root.contains(range.commonAncestorContainer) ? range.commonAncestorContainer : root
  if (around instanceof Text) {
    take(around)
  } else {
    const walker = document.createTreeWalker(around, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT)
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (range.intersectsNode(node))
        take(node)
    }
  }

  const base = root.getBoundingClientRect()
  const clips = new Map<Element, DOMRect | null>()
  /** The part of the viewport `element` shows: what every scrolled or clipped box around it in `root` leaves. */
  const clipOf = (element: Element): DOMRect | null => {
    if (element === root || !element.parentElement)
      return null
    if (clips.has(element))
      return clips.get(element) ?? null
    const outer = clipOf(element.parentElement)
    const own = style(element)
    let clip = outer
    if (own.overflowX !== 'visible' || own.overflowY !== 'visible') {
      const box = element.getBoundingClientRect()
      clip = outer ? intersect(outer, box) : box
    }
    clips.set(element, clip)
    return clip
  }
  const relative = (box: DOMRect): DrawnBox => ({ top: box.top - base.top, left: box.left - base.left, width: box.width, height: box.height })

  const last = lastLine(range, lines)
  const strips: DrawnBox[] = []
  for (const line of lines) {
    const right = line === last ? line.right : line.right + spaceWidth(line.font)
    const strip = new DOMRect(line.left, line.mid - line.height / 2 + GAP / 2, right - line.left, line.height - GAP)
    const clip = clipOf(line.block)
    const shown = clip ? intersect(clip, strip) : strip
    if (shown.width > 0 && shown.height > 0)
      strips.push(relative(shown))
  }
  const drawn: DrawnPicture[] = []
  for (const picture of pictures) {
    const box = picture.getBoundingClientRect()
    const clip = picture.parentElement ? clipOf(picture.parentElement) : null
    const shown = clip ? intersect(clip, box) : box
    if (shown.width === 0 || shown.height === 0)
      continue
    // The outline stands this far out of the box: its offset and its width.
    const out = -OUTLINE
    const inset = clip
      ? [clip.top - box.top, box.right - clip.right, box.bottom - clip.bottom, clip.left - box.left].map((side) => `${Math.max(out, side)}px`).join(' ')
      : `${out}px`
    drawn.push({ ...relative(box), radius: Number.parseFloat(style(picture).borderTopLeftRadius) || 0, clip: inset })
  }
  return { strips, pictures: drawn }
}

/**
 * The line the selection ends in, which it does not go on past: the one its
 * last selected letter stands on. None when it ends before a block's first
 * letter, as a triple click's does, which goes on past every line.
 */
function lastLine(range: Range, lines: Line[]): Line | null {
  const { endContainer, endOffset } = range
  if (!(endContainer instanceof Text) || endOffset === 0)
    return null
  const letter = document.createRange()
  letter.setStart(endContainer, endOffset - 1)
  letter.setEnd(endContainer, endOffset)
  const box = [...letter.getClientRects()].at(-1)
  if (!box)
    return null
  const mid = box.top + box.height / 2
  return lines.find((line) => Math.abs(line.mid - mid) < line.height / 2 && box.right <= line.right + 1 && box.left >= line.left - 1) ?? null
}

let measure: CanvasRenderingContext2D | null = null
const spaces = new Map<string, number>()

/** The width of a space in `font`. */
function spaceWidth(font: string): number {
  let width = spaces.get(font)
  if (width === undefined) {
    measure ??= document.createElement('canvas').getContext('2d')
    if (measure)
      measure.font = font
    width = measure?.measureText(' ').width ?? 0
    spaces.set(font, width)
  }
  return width
}

function intersect(a: DOMRect, b: DOMRect): DOMRect {
  const left = Math.max(a.left, b.left)
  const top = Math.max(a.top, b.top)
  return new DOMRect(left, top, Math.max(0, Math.min(a.right, b.right) - left), Math.max(0, Math.min(a.bottom, b.bottom) - top))
}
