/** How long a row the filter opened may take to appear, such as on a page that loads its list first. */
const APPEAR_TIMEOUT_MS = 3000

function rowIn(root: HTMLElement, label: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-setting="${CSS.escape(label)}"]`)
}

function flash(row: HTMLElement): void {
  row.scrollIntoView({ block: 'center' })
  // A class set again restarts nothing: take it off first, so a second find flashes again.
  row.classList.remove('setting-highlight')
  void row.offsetWidth
  row.classList.add('setting-highlight')
  row.addEventListener('animationend', () => row.classList.remove('setting-highlight'), { once: true })
}

/**
 * Scrolls the settings row labelled `label` under `root` into view and
 * flashes it, as System Settings shows a setting its search found: at once
 * when the row is there, or as soon as the page shows it. Gives up after a
 * few seconds, or when `signal` aborts, for another find or a closed page.
 */
export function highlightSetting(root: HTMLElement, label: string, signal: AbortSignal): void {
  const row = rowIn(root, label)
  if (row) {
    flash(row)
    return
  }
  const observer = new MutationObserver(() => {
    const appeared = rowIn(root, label)
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
