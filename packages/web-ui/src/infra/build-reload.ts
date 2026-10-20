/**
 * Loading the page again for the build its backend serves, at most once per
 * build: a cache in front of the backend that keeps serving an earlier page
 * would otherwise make the page load itself again and again.
 */

/**
 * Loads the page again, at the address it shows. A drafted message and the
 * reader's place outlive it, so this is the one reload of the page: for another
 * build, and for a part of the page that failed to load (`infra/lazy.ts`).
 */
export function reloadPage(): void {
  window.location.reload()
}

/** The `sessionStorage` key that keeps the build the page last loaded itself for. */
const RELOADED_FOR = 'demi.reloadedFor'

/**
 * Loads the page again with `reload` for `build`, unless it did so for that
 * build already; answers whether it did.
 */
export function reloadFor(build: string, reload: () => void): boolean {
  try {
    if (sessionStorage.getItem(RELOADED_FOR) === build) {
      return false
    }
    sessionStorage.setItem(RELOADED_FOR, build)
  } catch {
    // Without the record a reload could repeat without end, so the page
    // stays and says that it could not be updated.
    return false
  }
  reload()
  return true
}
