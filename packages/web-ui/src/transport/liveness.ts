/**
 * The liveness of a page's WebSockets to the backend, the synchronization
 * channel and each conversation socket (`web-application.md` § Liveness and
 * reconnection). The backend sends a heartbeat on a socket that has sent
 * nothing else for 30 seconds, so a socket that brings nothing for much
 * longer died without a close, as when a laptop slept and its network
 * dropped. A socket that closes, breaks or cannot be made is tried again
 * after waits that double.
 */

/** Two and a half of the backend's 30-second heartbeats: a socket silent this long is broken. */
export const SILENCE_MS = 75_000
/** The first wait before a socket connects again; each later one doubles, up to the longest. */
const FIRST_WAIT_MS = 1_000
const LONGEST_WAIT_MS = 30_000

/**
 * The wait before connecting again after `failures` (one or more)
 * consecutive connections ended or could not be made: a second, then twice
 * as long each time, up to 30 seconds, each shortened by a random part of up
 * to half, so that the pages of all users do not return at once after a
 * restart.
 */
export function reconnectWait(failures: number): number {
  const longest = Math.min(FIRST_WAIT_MS * 2 ** (failures - 1), LONGEST_WAIT_MS)
  return longest * (1 - Math.random() / 2)
}

/** A watch over one socket's silence. */
export interface SilenceWatch {
  /** The socket brought a message, a heartbeat included: its silence starts again. */
  heard(): void
  /** Ends the watch; nothing is called after it. */
  stop(): void
}

/**
 * Watches a socket from now on and calls `broken` once when it has brought
 * nothing for `SILENCE_MS`, counted from now or from the last `heard`. The
 * socket's owner stops the watch when it lets the socket go, whatever the
 * reason.
 */
export function watchSilence(broken: () => void): SilenceWatch {
  let timer: ReturnType<typeof setTimeout> | null = null
  const arm = () => {
    timer = setTimeout(() => {
      timer = null
      broken()
    }, SILENCE_MS)
  }
  arm()
  return {
    heard() {
      // A watch that fired or stopped stays so.
      if (timer === null) {
        return
      }
      clearTimeout(timer)
      arm()
    },
    stop() {
      if (timer !== null) {
        clearTimeout(timer)
      }
      timer = null
    },
  }
}
