/** Whether an element's one line of text runs past its box, so the box cuts it. */
export function isTextCut(element: HTMLElement): boolean {
  return element.scrollWidth > element.clientWidth
}

/**
 * Whether a `TruncatedText` with `keepsStart`, by its root, shows less than its
 * whole text: its text's box has wrapped off its one line, or is cut on it.
 */
export function isKeptStartCut(root: HTMLElement): boolean {
  const line = root.lastElementChild
  const box = line?.lastElementChild
  const shown = box?.lastElementChild
  if (!(line instanceof HTMLElement) || !(box instanceof HTMLElement) || !(shown instanceof HTMLElement)) {
    return false
  }
  return box.getBoundingClientRect().top >= line.getBoundingClientRect().bottom || isTextCut(shown)
}
