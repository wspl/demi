/**
 * The least room left above a block brought into view from its start, so it
 * reads as the first thing shown rather than cut by the view's edge.
 */
export const REVEAL_MARGIN_PX = 16

/**
 * How far below the top of the view a block brought into view starts
 * (`product.md` § Finding a conversation): a block that fits the view stands
 * in its middle; one taller than the view starts near its top, so the reader
 * reads it from its first line, as a browser brings a long section's heading
 * into view. Every reveal places its block by this rule: a search result, a
 * notice's call, a receipt's block, a link to a line.
 */
export function revealOffset(blockHeight: number, viewHeight: number): number {
  return Math.max(REVEAL_MARGIN_PX, (viewHeight - blockHeight) / 2)
}

/** The nearest ancestor of `element` that scrolls vertically. */
function scroller(element: HTMLElement): HTMLElement | null {
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    const { overflowY } = getComputedStyle(parent)
    if ((overflowY === 'auto' || overflowY === 'scroll') && parent.scrollHeight > parent.clientHeight)
      return parent
  }
  return null
}

/** Scrolls the element's scroller so the element stands where `revealOffset` places it. */
export function revealElement(element: HTMLElement): void {
  const parent = scroller(element)
  if (!parent)
    return
  const view = parent.getBoundingClientRect()
  const box = element.getBoundingClientRect()
  parent.scrollTop += box.top - view.top - revealOffset(box.height, parent.clientHeight)
}
