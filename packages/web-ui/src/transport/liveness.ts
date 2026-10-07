/**
 * The liveness of a page's WebSockets to the backend: the synchronization
 * channel, each conversation socket and each live view of the conversation
 * browser (`web-application.md` § Liveness and reconnection). The far end of
 * each sends a heartbeat on a socket that has sent nothing else for a while,
 * 30 seconds at most, so a socket that brings nothing for much longer died
 * without a close, as when a laptop slept and its network dropped. A socket
 * that closes, breaks or cannot be made is tried again after waits that
 * double. Timers stop while a laptop sleeps, so when the page becomes visible
 * again or comes back online, this module checks every socket at once.
 */
import { defaultDocument, defaultWindow, useEventListener, useOnline } from '@vueuse/core'

/** Two and a half of the longest heartbeat, the backend's 30 seconds: a socket silent this long is broken. */
const SILENCE_MS = 75_000
/** The first wait before a socket connects again; each later one doubles, up to the longest. */
const FIRST_WAIT_MS = 1_000
const LONGEST_WAIT_MS = 30_000
/** The shortest wait while the backend restarts; each is up to twice as long. */
const RESTART_WAIT_MS = 1_000

/** The page's open sockets, each by the check of its silence that the page's return makes. */
const watched = new Set<() => void>()
/** The page's closed sockets, each by what connects it at once instead of after its wait. */
const waiting = new Set<() => void>()

/**
 * The wait before connecting again after `failures` (one or more)
 * consecutive connections ended or could not be made: a second, then twice
 * as long each time, up to 30 seconds, each shortened by a random part of up
 * to half, so that the pages of all users do not return at once after a
 * restart.
 */
function reconnectWait(failures: number): number {
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
 * nothing for `SILENCE_MS`, counted from now or from the last `heard`: when
 * the watch's timer ends, or at the page's return when the clock shows that
 * much silence while the timer slept. At a return that finds less, the
 * watch counts on from the last message by the clock. The socket's owner
 * stops the watch when it lets the socket go, whatever the reason.
 */
export function watchSilence(broken: () => void): SilenceWatch {
  // The wall clock, which goes on while the machine sleeps.
  let heardAt = Date.now()
  let timer: ReturnType<typeof setTimeout> | null = null
  const stop = () => {
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = null
    watched.delete(check)
  }
  const fire = () => {
    stop()
    broken()
  }
  const arm = (ms: number) => {
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = setTimeout(fire, ms)
  }
  const check = () => {
    const silent = Date.now() - heardAt
    if (silent >= SILENCE_MS) {
      fire()
      return
    }
    arm(SILENCE_MS - silent)
  }
  arm(SILENCE_MS)
  watched.add(check)
  return {
    heard() {
      // A watch that fired or stopped stays so.
      if (timer === null) {
        return
      }
      heardAt = Date.now()
      arm(SILENCE_MS)
    },
    stop,
  }
}

/** A closed socket's wait before it connects again. */
export interface ReconnectWait {
  /** Ends the wait without connecting; nothing is called after it. */
  cancel(): void
}

/**
 * Calls `connect` once: after the wait for `failures` (one or more)
 * consecutive connections that ended or could not be made, or at the page's
 * return if that comes first. The socket's owner cancels the wait when it
 * connects at once for a reason of its own or lets the socket go.
 */
export function waitToReconnect(failures: number, connect: () => void): ReconnectWait {
  return wait(reconnectWait(failures), connect)
}

/** Calls `connect` once, after `ms` or at the page's return. */
function wait(ms: number, connect: () => void): ReconnectWait {
  const cancel = () => {
    clearTimeout(timer)
    waiting.delete(now)
  }
  const now = () => {
    cancel()
    connect()
  }
  const timer = setTimeout(now, ms)
  waiting.add(now)
  return { cancel }
}

/**
 * Calls `connect` once while the backend restarts, which it said as it
 * closed the socket: after one to two seconds, at a random point, so that the
 * pages of all users do not return at once; or at the page's return if that
 * comes first (`web-application.md` § A page of another build).
 */
export function waitWhileRestarting(connect: () => void): ReconnectWait {
  return wait(RESTART_WAIT_MS * (1 + Math.random()), connect)
}

/**
 * The page came back: it became visible again, or came back online. Each
 * open socket that has brought nothing for `SILENCE_MS` by the clock is
 * broken at once, and each other one watched for the rest of that time by
 * the clock. Then each closed socket connects without waiting for the rest
 * of its wait, one just broken among them when its owner starts the wait as
 * the break reaches it. An owner that learns of the break through a promise,
 * as a conversation that still opens does, starts its wait once the promises
 * due have run, so the closed sockets are swept once more when the task the
 * return came in has ended.
 */
export function pageReturned(): void {
  for (const check of [...watched]) {
    check()
  }
  connectWaiting()
  // Left to run: it only connects the sockets that wait by then.
  setTimeout(connectWaiting, 0)
}

/**
 * The page's path to a Host changed, as when a direct channel opened or
 * failed (`direct-channel.md` § Choosing the path): each socket that waits
 * to connect again, such as a view its owner just ended for the move,
 * connects now on the new path. Moving costs no reconnect wait.
 */
export function reconnectNow(): void {
  connectWaiting()
}

/** Connects each closed socket now instead of after the rest of its wait. */
function connectWaiting(): void {
  for (const now of [...waiting]) {
    now()
  }
}

// For the page's lifetime; without a document, as in a test, there is no return to follow.
useEventListener(defaultDocument, 'visibilitychange', () => {
  if (defaultDocument?.visibilityState === 'visible') {
    pageReturned()
  }
})
useEventListener(defaultWindow, 'online', pageReturned)

/**
 * Whether the user's browser reports a network, which the connection banner
 * says when it does not (`web-application.md` § A page of another build).
 */
export const browserOnline = useOnline()
