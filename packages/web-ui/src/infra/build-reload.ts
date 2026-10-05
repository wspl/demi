/**
 * Loading the page again for the build its backend serves
 * (`web-application.md` § A page of another build), at most once per build:
 * a cache in front of the backend that keeps serving an earlier page would
 * otherwise make the page load itself again and again.
 */

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
