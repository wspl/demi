/**
 * Where a comment stands in a reply's text block (`comments-and-ask.md`
 * § Sending them): character offsets into the block's rendered text, so a
 * mark survives the reply being drawn again, as a theme change re-coloring
 * its code does, where a kept DOM Range would not. Text inside a control
 * set into the reply, such as a code block's Copy, is not the reply's and is
 * skipped.
 */
export interface TextAnchor {
  start: number
  end: number
}

/** Whether a text node belongs to the reply, not to a control set into it. */
function ownText(node: Node): boolean {
  const parent = node.parentElement
  return parent !== null && parent.closest('button, [data-no-anchor]') === null
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
  // Text nodes come in document order: those before the boundary count whole, the one it falls in up to it.
  for (const node of textNodes(root)) {
    if (node === container)
      return count + offset
    if (!probe.intersectsNode(node))
      break
    count += node.length
  }
  return count
}

/** The anchor of `range` in `root`; null when it leaves the reply or holds no text. */
export function anchorOf(root: HTMLElement, range: Range): TextAnchor | null {
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer))
    return null
  const start = offsetOf(root, range.startContainer, range.startOffset)
  const end = offsetOf(root, range.endContainer, range.endOffset)
  return end > start ? { start, end } : null
}

/** The live range of `anchor` in `root` as it renders now; null when the text is shorter. */
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
export function rangeHas(range: Range, x: number, y: number): boolean {
  for (const rect of range.getClientRects()) {
    if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom)
      return true
  }
  return false
}

/** The anchor of the first place `text` stands in `root`'s own text, as a gallery specimen names what it marks. */
export function anchorOfText(root: HTMLElement, text: string): TextAnchor | null {
  const start = textNodes(root).map((node) => node.data).join('').indexOf(text)
  return start < 0 ? null : { start, end: start + text.length }
}
