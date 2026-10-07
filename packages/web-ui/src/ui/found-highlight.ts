/** How long the element a find opened may take to appear, such as on a page that loads its list first. */
const APPEAR_TIMEOUT_MS = 3000

function flash(element: HTMLElement): void {
  element.scrollIntoView({ block: 'center' })
  // A class set again restarts nothing: take it off first, so a second find flashes again.
  element.classList.remove('found-highlight')
  void element.offsetWidth
  element.classList.add('found-highlight')
  element.addEventListener('animationend', () => element.classList.remove('found-highlight'), { once: true })
}

/**
 * Scrolls the element under `root` that `selector` names into view and
 * flashes it, as System Settings shows a setting its search found and Slack
 * a message a search opened: at once when the element is there, or as soon
 * as the page shows it. Gives up after a few seconds, or when `signal`
 * aborts, for another find or a closed page.
 */
export function highlightFound(root: HTMLElement, selector: string, signal: AbortSignal): void {
  const found = root.querySelector<HTMLElement>(selector)
  if (found) {
    flash(found)
    return
  }
  const observer = new MutationObserver(() => {
    const appeared = root.querySelector<HTMLElement>(selector)
    if (appeared) {
      stop()
      flash(appeared)
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
