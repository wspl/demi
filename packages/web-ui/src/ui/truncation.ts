/** Whether an element's one line of text runs past its box, so the box cuts it. */
export function isTextCut(element: HTMLElement): boolean {
  return element.scrollWidth > element.clientWidth
}
