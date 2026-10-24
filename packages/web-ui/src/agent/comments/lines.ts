/**
 * Where a comment's box or a side question's card stands in a reply: under
 * the line that holds the end of its words, inside the reply, the rest of the
 * paragraph going on under it
 * (`docs/decisions/0092-a-remark-stands-under-its-line.md`).
 *
 * A remark's place is an element of its own, set before the first character
 * of the next line. A block set there breaks the line where it already broke,
 * so the lines above it keep their layout:
 *
 * ```text
 *   …runs under ‾the same timeout‾ as every other API call, so a
 *   ┌ place ──────────────────────────────────────────────────┐
 *   └─────────────────────────────────────────────────────────┘
 *   40 MB video on a 3G connection is cut off part of the way.
 * ```
 *
 * - The text node that holds the next line's first character is split there.
 *   Inside an element of the line, such as a link or bold words, that element
 *   is split too, into two halves alike, so the place stands between the lines
 *   and never inside a link. Inline math, which a split would break, is passed
 *   whole: the place goes after it.
 * - Words that end on their block's last line put it after the block; inside a
 *   list item, at the item's end, so it keeps the item's indent.
 * - What cannot be cut, a code block, a table, a display formula or a row of
 *   pictures, takes it after the whole of it, outside its scroller.
 * - Places at one point stand in the order of the remarks given.
 *
 * A place stays while its point is where it would be put. When the reply's
 * width changes, the lines wrap again: every place is taken out, the text and
 * elements it split joined again, and each put back (`clearPlaces`).
 */

/** The attribute that marks a place. It is not the reply's text, which anchors skip. */
export const PLACE_ATTRIBUTE = 'data-remark-place'

/** A new, empty place, which the caller fills (a teleport's target). */
export function createPlace(): HTMLElement {
  const place = document.createElement('div')
  place.setAttribute(PLACE_ATTRIBUTE, '')
  place.dataset.noAnchor = ''
  return place
}

function isPlace(node: Node | null): node is HTMLElement {
  return node instanceof HTMLElement && node.hasAttribute(PLACE_ATTRIBUTE)
}

/** Whether `element` lays out as a block of its own, not inside a line. */
function isBlock(element: Element): boolean {
  const display = getComputedStyle(element).display
  return !display.startsWith('inline') && display !== 'contents' && display !== 'none'
}

/** What a line cannot be cut inside, with what scrolls or frames it. */
const UNCUT = '.code-block, pre, .table-scroll, table, .katex-display, .math-display, .media-run'

/** What a place cannot split, though it stands in a line. */
const UNSPLIT = '.katex, .math-inline, svg, button'

/** Marks the second half of an element a place split, which taking it out joins again. */
const SPLIT_ATTRIBUTE = 'data-remark-split'

/** Where a place goes: before `before` in `parent`, at its end for null. */
interface Point {
  parent: Node
  before: Node | null
}

/** The point right after `node`. */
function after(node: Node): Point {
  return { parent: node.parentNode!, before: node.nextSibling }
}

/** Whether `element` lays its children out side by side, as items, not lines. */
function laysOutItems(element: Element): boolean {
  return /(flex|grid)/.test(getComputedStyle(element).display)
}

/**
 * The block that holds `node` and lays out its lines, within `root`. An item
 * of a flex or grid box, such as a lone picture's link, is no such block: the
 * box is, and a place set in it would stand beside the item.
 */
function blockOf(root: HTMLElement, node: Node): Element {
  for (let at: Node | null = node instanceof Element ? node : node.parentNode; at && at !== root; at = at.parentNode) {
    if (at instanceof Element && isBlock(at) && !(at.parentElement && at.parentElement !== root && laysOutItems(at.parentElement)))
      return at
  }
  return root
}

/** The child of `block` that holds `node`. */
function childOf(block: Node, node: Node): Node {
  let at = node
  while (at.parentNode && at.parentNode !== block)
    at = at.parentNode
  return at
}

/** Whether `node` has no text of `block`'s before it, within `child` of `block`. */
function startsChild(child: Node, node: Node): boolean {
  for (let at: Node | null = node; at && at !== child; at = at.parentNode) {
    if (at.previousSibling && (at.previousSibling.textContent ?? '').trim() !== '')
      return false
  }
  return true
}

/** The point right after `node`, splitting the elements between it and `block`. */
function splitAfter(block: Node, node: Node): Point {
  let at = node
  while (at.parentNode && at.parentNode !== block && at === at.parentNode.lastChild)
    at = at.parentNode
  if (at.parentNode === block || !at.nextSibling)
    return after(at)
  return { parent: block, before: splitBefore(block, at.nextSibling) }
}

/** The outermost element around `node`, below `top`, that matches `selector`. */
function outermost(node: Node, top: Node, selector: string): Element | null {
  let found: Element | null = null
  for (let at = node instanceof Element ? node : node.parentElement; at && at !== top; at = at.parentElement) {
    if (at.matches(selector))
      found = at
  }
  return found
}

/**
 * Splits each element between `node` and `block` before `node`, so `node`
 * starts a child of `block`, and returns that child. A part with nothing
 * before it in its element is not split: the element itself starts there.
 */
function splitBefore(block: Node, node: Node): Node {
  let at = node
  while (at.parentNode && at.parentNode !== block) {
    const parent = at.parentNode as Element
    if (at === parent.firstChild) {
      at = parent
      continue
    }
    const half = parent.cloneNode(false) as Element
    half.setAttribute(SPLIT_ATTRIBUTE, '')
    for (let move: Node | null = at; move;) {
      const next: Node | null = move.nextSibling
      half.appendChild(move)
      move = next
    }
    parent.after(half)
    at = half
  }
  return at
}

/**
 * The point under the line of `range`'s end, in the reply text `root`, or null
 * for a range the page does not lay out.
 */
function pointUnder(root: HTMLElement, range: Range): Point | null {
  const rects = [...range.getClientRects()].filter((rect) => rect.width > 0 || rect.height > 0)
  const last = rects.at(-1)
  if (!last)
    return null
  const lower = (rect: DOMRect) => rect.height > 0 && rect.top > last.top + last.height / 2
  const end = range.endContainer
  const uncut = outermost(end, root, UNCUT)
  if (uncut)
    return after(uncut)
  const block = blockOf(root, end)
  // The first node after the end, in document order within the block.
  let start: Node | null
  let offset = 0
  if (end instanceof Text) {
    start = end
    offset = range.endOffset
  } else {
    start = end.childNodes[range.endOffset] ?? null
    if (!start) {
      let at: Node | null = end
      while (at && at !== block && !at.nextSibling)
        at = at.parentNode
      start = at && at !== block ? at.nextSibling : null
    }
  }
  while (isPlace(start))
    start = start.nextSibling
  const walker = document.createTreeWalker(block, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => (isPlace(node) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  })
  /** The point before `node`, whose first character or picture starts the next line. */
  const before = (node: Node, at: number): Point => {
    if (node.parentNode === block || node === block) {
      if (node instanceof Text && at > 0)
        return { parent: block, before: node.splitText(at) }
      return { parent: node.parentNode!, before: node }
    }
    // Inside an element of the line: before it when it starts the line, else
    // between the halves it is split into; after one a split would break.
    const child = childOf(block, node)
    if (at === 0 && startsChild(child, node))
      return { parent: block, before: child }
    const whole = outermost(node, block, UNSPLIT)
    if (whole)
      return splitAfter(block, whole)
    const next = node instanceof Text && at > 0 ? node.splitText(at) : node
    return { parent: block, before: splitBefore(block, next) }
  }
  for (let node: Node | null = start; node; node = walker.nextNode()) {
    if (node === start)
      walker.currentNode = node
    if (node instanceof Element) {
      if (node !== block && isBlock(node))
        return { parent: node.parentNode!, before: node }
      if (node.matches('img, [role="img"]') && lower(node.getBoundingClientRect()))
        return before(node, 0)
      continue
    }
    if (!(node instanceof Text))
      continue
    const from = node === start ? offset : 0
    if (from >= node.length)
      continue
    // A node wholly on the line is passed at once.
    const whole = document.createRange()
    whole.setStart(node, from)
    whole.setEnd(node, node.length)
    if (![...whole.getClientRects()].some(lower))
      continue
    const char = document.createRange()
    for (let at = from; at < node.length; at++) {
      if (/\s/.test(node.data[at]!))
        continue
      char.setStart(node, at)
      char.setEnd(node, at + 1)
      if (lower(char.getBoundingClientRect()))
        return before(node, at)
    }
  }
  // The words end on the block's last line.
  if (block === root || block.matches('li, td, th, blockquote'))
    return { parent: block, before: null }
  return after(block)
}

/** Past the places already at `point`: the content that follows them. */
function pastPlaces(point: Point): Point {
  let before = point.before
  while (isPlace(before))
    before = before.nextSibling
  return { parent: point.parent, before }
}

/** The places that stand at `point`, right before the content it is past. */
function placesAt(point: Point): HTMLElement[] {
  const run: HTMLElement[] = []
  for (let at = point.before ? point.before.previousSibling : point.parent.lastChild; isPlace(at); at = at.previousSibling)
    run.unshift(at)
  return run
}

/**
 * Where `place` goes among the places at `point`, by `rank`: before the first
 * that ranks after it, else after them all.
 */
function slotFor(place: HTMLElement, point: Point, rank: ReadonlyMap<HTMLElement, number>): Point {
  const own = rank.get(place) ?? Infinity
  const later = placesAt(point).find((other) => other !== place && (rank.get(other) ?? Infinity) > own)
  return later ? { parent: point.parent, before: later } : point
}

/** Whether `place` stands at `point`, in its rank among the places there. */
function standsAt(place: HTMLElement, point: Point, rank: ReadonlyMap<HTMLElement, number>): boolean {
  const run = placesAt(point)
  const index = run.indexOf(place)
  if (index < 0)
    return false
  const own = rank.get(place) ?? Infinity
  const rankOf = (other: HTMLElement | undefined) => (other ? rank.get(other) ?? Infinity : undefined)
  const before = rankOf(run[index - 1])
  const after = rankOf(run[index + 1])
  return (before === undefined || before < own) && (after === undefined || after > own)
}

/** A remark to place: its place, and its words' range as the reply holds them now. */
export interface Remark {
  place: HTMLElement
  /** The reply's rendered text that holds the words, or null for a reply not drawn. */
  root: HTMLElement | null
  /**
   * The words' range, read anew each time it is asked for: placing splits and
   * joins the text, which moves a live range read before.
   */
  range: () => Range | null
}

/**
 * Puts each place under the line of its range's end. Places at one point stand
 * in the order of `remarks`. A place without a range, as one whose words the
 * reply no longer holds, is taken out. A place already where it would be put
 * is left as it is, so a placing that finds them placed changes nothing.
 */
export function placeRemarks(remarks: readonly Remark[]): void {
  const rank = new Map(remarks.map(({ place }, index) => [place, index]))
  for (const { place, root, range } of remarks) {
    const words = root ? range() : null
    const point = words ? pointUnder(root!, words) : null
    if (!point) {
      takeOut(place)
      continue
    }
    if (standsAt(place, pastPlaces(point), rank))
      continue
    takeOut(place)
    // Taking it out may have joined what the point stands in: find it again.
    const again = range()
    const target = again ? pointUnder(root!, again) : null
    if (target) {
      const at = slotFor(place, pastPlaces(target), rank)
      at.parent.insertBefore(place, at.before)
    }
  }
  // A point found and then not taken, as a place moved on once the text it
  // left joined, leaves halves no place stands between: join them again.
  for (const root of new Set(remarks.map(({ root }) => root))) {
    for (const half of root?.querySelectorAll(`[${SPLIT_ATTRIBUTE}]`) ?? []) {
      if (!isPlace(half.previousSibling))
        join(half.previousSibling, half)
    }
  }
}

/** Joins `second` into `first` where a place had split them, and their insides alike. */
function join(first: Node | null, second: Node | null): void {
  if (first instanceof Text && second instanceof Text) {
    first.appendData(second.data)
    second.remove()
  } else if (first instanceof Element && second instanceof Element && second.hasAttribute(SPLIT_ATTRIBUTE) && first.tagName === second.tagName) {
    const last = first.lastChild
    const next = second.firstChild
    first.append(...second.childNodes)
    second.remove()
    join(last, next)
  }
}

/** Takes `place` out, joining the text and the elements it split. */
function takeOut(place: HTMLElement): void {
  if (!place.isConnected)
    return
  const before = place.previousSibling
  const next = place.nextSibling
  place.remove()
  join(before, next)
}

/**
 * `range` in pieces around the places within it, so a mark drawn on the words
 * draws none on the boxes standing between their lines.
 */
export function aroundPlaces(range: Range): Range[] {
  const root = range.commonAncestorContainer
  if (!(root instanceof Element))
    return [range]
  const pieces: Range[] = []
  let piece = range.cloneRange()
  for (const place of root.querySelectorAll(`[${PLACE_ATTRIBUTE}]`)) {
    if (!range.intersectsNode(place) || place.contains(range.startContainer) || place.contains(range.endContainer))
      continue
    piece.setEndBefore(place)
    if (!piece.collapsed)
      pieces.push(piece)
    piece = range.cloneRange()
    piece.setStartAfter(place)
  }
  if (!piece.collapsed)
    pieces.push(piece)
  return pieces
}

/** Takes every place in `places` out, as the lines are about to wrap again. */
export function clearPlaces(places: Iterable<HTMLElement>): void {
  for (const place of places)
    takeOut(place)
}
