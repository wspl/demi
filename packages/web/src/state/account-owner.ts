/**
 * Which account the browser's data is: the one fact every write of an
 * account's data asks first (`holds`), so that a page whose account signed
 * out, or gave the browser to another, in this tab or another, writes nothing
 * of it. `./account-data` sets and clears it; it lives apart so that the
 * stores that write can read it without importing what forgets them.
 */

/** The `localStorage` key that names the account the browser's data is. */
const OWNER_KEY = 'demi.account'

/** The account the browser's data is; none when it holds none. */
export function heldAccount(): string | null {
  try {
    return localStorage.getItem(OWNER_KEY)
  } catch {
    // Storage the web browser refuses holds nothing of any account.
    return null
  }
}

/** Whether the browser's data is `userId`'s, which only then may be written. */
export function holds(userId: string): boolean {
  return heldAccount() === userId
}

/** Whether the browser's data is any account's, for the data that names none. */
export function holdsAny(): boolean {
  return heldAccount() !== null
}

export function setOwner(userId: string | null): void {
  try {
    if (userId === null) {
      localStorage.removeItem(OWNER_KEY)
    } else {
      localStorage.setItem(OWNER_KEY, userId)
    }
  } catch {
    // Storage the web browser refuses keeps nothing to own.
  }
}

/**
 * Calls `handler` with the new owner whenever another tab of this browser
 * changes it, as its sign-out, its session's end or another account's
 * sign-in does; the change this tab makes is not heard.
 */
export function onOwnerChange(handler: (owner: string | null) => void): () => void {
  const listener = (event: StorageEvent) => {
    // A `clear()` of the whole storage names no key.
    if (event.key === OWNER_KEY || event.key === null) {
      handler(heldAccount())
    }
  }
  window.addEventListener('storage', listener)
  return () => window.removeEventListener('storage', listener)
}
