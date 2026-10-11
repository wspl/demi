import { revealElement } from './reveal'

/** How long the element a find opened may take to appear, such as on a page that loads its list first. */
const APPEAR_TIMEOUT_MS = 3000

function flash(element: HTMLElement, scroll: boolean): void {
  if (scroll)
    revealElement(element)
  // A class set again restarts nothing: take it off first, so a second find flashes again.
  element.classList.remove('found-highlight')
  void element.offsetWidth
  element.classList.add('found-highlight')
  // Only the mark's own end: a row's content animating inside it ends the mark early otherwise.
  const ended = (event: AnimationEvent) => {
    if (event.animationName !== 'found-highlight')
      return
    element.classList.remove('found-highlight')
    element.removeEventListener('animationend', ended)
  }
  element.addEventListener('animationend', ended)
}

/**
 * Scrolls the element under `root` that `selector` names into view, as
 * every reveal places a block (`reveal.ts`), and marks it for a moment, as
 * System Settings shows a setting its search found and Slack a message a
 * search opened: at once when the element is there, or as soon as the page
 * shows it. A list that places its rows itself, as the virtualized
 * transcript does, has brought it into view already and passes `scroll:
 * false`. Gives up after a few seconds, or when `signal` aborts, for
 * another find or a closed page.
 */
export function highlightFound(
  root: HTMLElement,
  selector: string,
  signal: AbortSignal,
  { scroll = true }: { scroll?: boolean } = {},
): void {
  const found = root.querySelector<HTMLElement>(selector)
  if (found) {
    flash(found, scroll)
    return
  }
  const observer = new MutationObserver(() => {
    const appeared = root.querySelector<HTMLElement>(selector)
    if (appeared) {
      stop()
      flash(appeared, scroll)
    }
  })
  const timer = setTimeout(stop, APPEAR_TIMEOUT_MS)
  function stop(): void {
    observer.disconnect()
    clearTimeout(timer)
    signal.removeEventListener('abort', stop)
  }
  observer.observe(root, { childList: true, subtree: true })
  signal.addEventListener('abort', stop, { once: true })
}
