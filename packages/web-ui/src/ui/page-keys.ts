import { shortcutSet, withCommandKey } from './shortcut'

/**
 * The keys the page answers while it has a selection and the focus, acted on
 * for a field that stands for the selection from a frame of its own
 * (`FramedField.vue`) and holds no words yet: with them the field is as if
 * it were not there, until the user types in it.
 *
 * - Shift with an arrow extends the selection, as the system's keys for a word
 *   or a line's and the page's edges do with it.
 * - Select All does what the browser's does on the page.
 * - Space, Page Up, Page Down, Home, End and the up and down arrows scroll
 *   what holds the selection.
 * - Tab and Shift-Tab move the focus to the next or the previous control from
 *   the selection; none in `field`, which the page would not have.
 *
 * Copy, which the field answers itself while it selects nothing of its own,
 * is not here.
 */
export function answerForPage(event: KeyboardEvent, field: Element): boolean {
  const selection = window.getSelection()
  if (!selection || selection.rangeCount === 0)
    return false
  const mac = shortcutSet() === 'mac'
  const { key, shiftKey, altKey, ctrlKey, metaKey } = event

  if (shiftKey && key.startsWith('Arrow')) {
    const forward = key === 'ArrowRight' || key === 'ArrowDown'
    const across = key === 'ArrowLeft' || key === 'ArrowRight'
    const granularity: Granularity = mac
      ? metaKey ? (across ? 'lineboundary' : 'documentboundary') : altKey ? (across ? 'word' : 'paragraphboundary') : (across ? 'character' : 'line')
      : ctrlKey ? (across ? 'word' : 'paragraphboundary') : (across ? 'character' : 'line')
    if (mac ? ctrlKey : altKey || metaKey)
      return false
    selection.modify('extend', forward ? 'forward' : 'backward', granularity)
    return true
  }
  if (!mac && shiftKey && (key === 'Home' || key === 'End') && !altKey && !metaKey) {
    selection.modify('extend', key === 'End' ? 'forward' : 'backward', ctrlKey ? 'documentboundary' : 'lineboundary')
    return true
  }
  if (withCommandKey(event) && key.toLowerCase() === 'a' && !shiftKey && !altKey) {
    // The browser's own command, which selects what its Select All does on this page.
    document.execCommand('selectAll')
    return true
  }
  if (key === 'Tab' && !altKey && !ctrlKey && !metaKey)
    return moveFocus(selection.getRangeAt(0), shiftKey ? -1 : 1, field)
  if (altKey || ctrlKey || (metaKey && !(mac && (key === 'ArrowUp' || key === 'ArrowDown'))))
    return false
  if (shiftKey && key !== ' ')
    return false
  return scroll(selection.getRangeAt(0), key, shiftKey, metaKey)
}

type Granularity = 'character' | 'word' | 'line' | 'lineboundary' | 'paragraphboundary' | 'documentboundary'

/** Chrome's step for an arrow key, in pixels. */
const ARROW_STEP = 40
/** The part of a scroller's height a page key moves it by, Chrome's, which keeps a little of the page before in sight. */
const PAGE_PART = 0.875

function scroll(range: Range, key: string, shiftKey: boolean, metaKey: boolean): boolean {
  const scroller = scrollerOf(range.startContainer)
  const page = scroller.clientHeight * PAGE_PART
  let by: number
  if (key === ' ')
    by = shiftKey ? -page : page
  else if (key === 'PageDown')
    by = page
  else if (key === 'PageUp')
    by = -page
  else if (key === 'Home' || (metaKey && key === 'ArrowUp'))
    by = -scroller.scrollTop
  else if (key === 'End' || (metaKey && key === 'ArrowDown'))
    by = scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop
  else if (key === 'ArrowDown')
    by = ARROW_STEP
  else if (key === 'ArrowUp')
    by = -ARROW_STEP
  else
    return false
  scroller.scrollBy({ top: by, behavior: 'smooth' })
  return true
}

/** The nearest box around `node` that scrolls up and down, else the page. */
function scrollerOf(node: Node): Element {
  for (let at = node instanceof Element ? node : node.parentElement; at; at = at.parentElement) {
    const { overflowY } = getComputedStyle(at)
    if ((overflowY === 'auto' || overflowY === 'scroll') && at.scrollHeight > at.clientHeight)
      return at
  }
  return document.scrollingElement ?? document.documentElement
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"]), [contenteditable="true"]'

/** Focuses the first control after the selection, or the last before it, as Tab from a selection does. */
function moveFocus(range: Range, step: 1 | -1, field: Element): boolean {
  const controls = [...document.querySelectorAll<HTMLElement>(FOCUSABLE)]
    .filter((control) => !field.contains(control) && control.checkVisibility())
  const from = step === 1 ? range.endContainer : range.startContainer
  const after = (control: HTMLElement) => (from.compareDocumentPosition(control) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0 && !control.contains(from)
  const next = step === 1 ? controls.find(after) : controls.findLast((control) => !after(control) && !control.contains(from))
  if (!next)
    return false
  next.focus()
  return true
}
