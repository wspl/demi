/**
 * Where a comment stands in a reply's text block: character offsets into the
 * block's rendered text, so a mark survives the reply being drawn again, as a
 * theme change re-coloring its code does, where a kept DOM Range would not.
 *
 * Text inside a control set into the reply, such as a code block's Copy, is not
 * the reply's and is skipped.
 */
export interface TextAnchor {
  start: number
  end: number
}

/**
 * A picture of a reply as it renders: an image, or the box that stands in a
 * picture's place while it loads or its copy is made (`imagePlaceholderHtml`),
 * which is a picture too, so the pictures after it keep their places as it
 * loads.
 */
const PICTURE = 'img, [role="img"]'

/**
 * Whether a text node belongs to the reply's words: not to a control set
 * into it, nor to a picture's stand-in, whose alt text goes as its file loads.
 */
function ownText(node: Node): boolean {
  const parent = node.parentElement
  return parent !== null && parent.closest(`button, [data-no-anchor], ${PICTURE}`) === null
}

function textNodes(root: HTMLElement): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => (ownText(node) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT),
  })
  const nodes: Text[] = []
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node instanceof Text)
      nodes.push(node)
  }
  return nodes
}

/** The offset in `root`'s own text of a boundary point of a range. */
function offsetOf(root: HTMLElement, container: Node, offset: number): number {
  const probe = document.createRange()
  probe.selectNodeContents(root)
  probe.setEnd(container, offset)
  let count = 0
  // Text nodes come in document order: those before the boundary count whole,
  // the one it falls in up to it.
  for (const node of textNodes(root)) {
    if (node === container)
      return count + offset
    if (!probe.intersectsNode(node))
      break
    count += node.length
  }
  return count
}

/**
 * The anchor of `range` in `root`; null when it leaves the reply or holds no
 * text.
 */
export function anchorOf(root: HTMLElement, range: Range): TextAnchor | null {
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer))
    return null
  const start = offsetOf(root, range.startContainer, range.startOffset)
  const end = offsetOf(root, range.endContainer, range.endOffset)
  return end > start ? { start, end } : null
}

/**
 * The live range of `anchor` in `root` as it renders now; null when the text is
 * shorter.
 */
export function rangeOf(root: HTMLElement, anchor: TextAnchor): Range | null {
  const range = document.createRange()
  let count = 0
  let started = false
  for (const node of textNodes(root)) {
    const next = count + node.length
    if (!started && anchor.start < next) {
      range.setStart(node, anchor.start - count)
      started = true
    }
    if (started && anchor.end <= next) {
      range.setEnd(node, anchor.end - count)
      return range
    }
    count = next
  }
  return null
}

/** Whether the point lies on one of `range`'s line boxes. */
export function rangeHas(range: Range | StaticRangeInit, x: number, y: number): boolean {
  const live = liveOf(range)
  for (const rect of live?.getClientRects() ?? []) {
    if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom)
      return true
  }
  return false
}

/**
 * The anchor of the first place `text` stands in `root`'s own text, as a
 * gallery specimen names what it marks.
 */
export function anchorOfText(root: HTMLElement, text: string): TextAnchor | null {
  const start = textNodes(root).map((node) => node.data).join('').indexOf(text)
  return start < 0 ? null : { start, end: start + text.length }
}

/**
 * `range` as it stands: the same words, still where they were.
 *
 * A list's rows are moved in the page as a whole, as a window opening beside
 * them moves the list's ancestors. A move collapses every live Range inside
 * what moved, though its words are unchanged. Where the range stood, its nodes
 * and offsets, still holds them, so the marks are kept so and read through
 * this. Null when the nodes no longer hold those offsets.
 */
export function liveOf(range: Range | StaticRangeInit): Range | null {
  if (range instanceof Range)
    return range
  try {
    const live = document.createRange()
    live.setStart(range.startContainer, range.startOffset)
    live.setEnd(range.endContainer, range.endOffset)
    return live
  } catch {
    // The text changed under the words: the next measure finds them again.
    return null
  }
}

/**
 * Where `range` stands, which a move of its nodes in the page leaves as it is
 * (`liveOf`).
 */
export function staticOf(range: Range): StaticRangeInit {
  const { startContainer, startOffset, endContainer, endOffset } = range
  return { startContainer, startOffset, endContainer, endOffset }
}

/**
 * The pictures of a reply's text block `root` as it renders them, in order: its
 * images and the stand-ins of those still being read or missing, leaving out an
 * icon, which the page hides from assistive technology, and those of a control
 * or card set into it, as a side question's answer. A comment on a picture
 * names it by its place among them.
 */
export function picturesOf(root: HTMLElement): Element[] {
  return [...root.querySelectorAll(PICTURE)].filter((picture) => picture.closest('button, [data-no-anchor], [aria-hidden="true"]') === null)
}

/**
 * The offset in `root`'s own text where `element` stands: the length of the
 * reply's words before it.
 */
function offsetBefore(root: HTMLElement, element: Element): number {
  let count = 0
  for (const node of textNodes(root)) {
    if (element.compareDocumentPosition(node) & Node.DOCUMENT_POSITION_FOLLOWING)
      break
    count += node.length
  }
  return count
}

/**
 * Where the picture `picture` of `root` stands: its place among the block's
 * pictures, and `at`, the offset in the block's text where it stands, by which
 * its comment takes its place among the block's others in reading order. Null
 * for a picture the block does not hold.
 */
export function pictureAnchor(root: HTMLElement, picture: Element): (TextAnchor & { at: number }) | null {
  const place = picturesOf(root).indexOf(picture)
  return place < 0 ? null : { start: place, end: place + 1, at: offsetBefore(root, picture) }
}

/**
 * What a comment on `image` quotes for a reader without the picture: its
 * alt text, else the name of the file it shows, else its address's last
 * part, else *Image*.
 */
export function pictureQuote(image: HTMLImageElement): string {
  const named = image.closest('[data-attachment-image]')?.getAttribute('data-attachment-image')
    ?? image.src.split(/[/?#]/).filter(Boolean).at(-1)
  return image.alt.trim() || named?.trim() || 'Image'
}

/**
 * The picture `anchor` names in `root`, by its place among the block's
 * pictures, when it is still the picture `quote` names; null while it is
 * not drawn, as a picture whose copy is being made, or when the reply holds
 * another.
 */
export function pictureOf(root: HTMLElement, anchor: TextAnchor, quote: string): HTMLImageElement | null {
  const image = picturesOf(root)[anchor.start]
  return image instanceof HTMLImageElement && pictureQuote(image) === quote ? image : null
}
