/**
 * The liveness of a page's WebSockets to the backend: the synchronization
 * channel, each conversation socket and each live view of the conversation
 * browser (`web-application.md` § Liveness and reconnection). The far end of
 * each sends a heartbeat on a socket that has sent nothing else for a while,
 * 30 seconds at most, so a socket that brings nothing for much longer died
 * without a close, as when a laptop slept and its network dropped. A socket
 * whose opening handshake does not complete within 10 seconds cannot be made.
 * A socket that closes, breaks or cannot be made is tried again after waits
 * that double. Timers stop while a laptop sleeps, and a phone's system cuts
 * a hidden page's connections without a close, so when the page returns
 * (it becomes visible again, the browser shows it from its back-forward
 * cache, a frozen page resumes, or it comes back online), this module checks
 * every socket once the page has read what its sockets hold: what it read
 * may have waited there for minutes, so it asks each open socket whose far
 * end can answer for a `pong` within 5 seconds, and opens again at once each
 * socket still opening from before the page went away. The browser's word
 * that it went offline counts once it has held for a second: then the page
 * says it is offline, and closes no socket, since one that kept its
 * connection, as through a VPN's tunnel, goes on working; coming back online
 * is a return.
 * The page reports why it replaced a socket through its error reports.
 */
import { readonly, ref } from 'vue'
import { defaultDocument, defaultNavigator, defaultWindow, useDocumentVisibility, useEventListener } from '@vueuse/core'
import type { PagePing } from '@demicodes/protocol'
import { reportError } from '../infra/errors'
import { t } from './locales/text'

/** Two and a half of the longest heartbeat, the backend's 30 seconds: a socket silent this long is broken. */
const SILENCE_MS = 75_000
/** The first wait before a socket connects again; each later one doubles, up to the longest. */
const FIRST_WAIT_MS = 1_000
const LONGEST_WAIT_MS = 30_000
/** The shortest wait while the backend restarts; each is up to twice as long. */
const RESTART_WAIT_MS = 1_000
/**
 * How long the browser's word that it is offline must hold before it counts:
 * a system that re-evaluates its network, as macOS does when it changes its
 * primary interface, reports offline for a moment while every socket stays open.
 */
const OFFLINE_HOLD_MS = 1_000

/**
 * How long an opening handshake may take: a web browser waits minutes for
 * one that a proxy in front of a restarting backend holds.
 */
const HANDSHAKE_MS = 10_000

/** How long a socket has to answer the return's ping: a working one answers in well under a second. */
const PONG_MS = 5_000

/** Why the page let an open socket go that had not closed. */
type Break = 'silence' | 'return' | 'probe'

/** What the page's report says of each, after the socket's name. */
const BREAK_WORDS: Record<Break, string> = {
  silence: 'brought nothing for 75 seconds',
  return: 'had brought nothing for 75 seconds by the clock when the page returned',
  probe: 'brought no pong at return within 5 seconds',
}

/**
 * The page's open sockets, each by the check that the page's return makes:
 * of its silence, and then of its pong; it calls `judged` once, when the
 * socket is judged alive or broken.
 */
const watched = new Set<(judged: () => void) => void>()
/**
 * The page's sockets still opening, each by what abandons it at the
 * return when it began before the page went away.
 */
const opening = new Map<WebSocket, { since: number; abandon: () => void }>()
/** How many times the page went away: hidden, frozen, put in the back-forward cache or offline. */
let absences = 0
/** The number of the page's last ping, which each next one counts on from. */
let lastPing = 0

/**
 * The number of the page's next ping, a return's or a relay probe's: both
 * get their `pong` on the direct channel's signaling socket, so one
 * numbering keeps each answer to the one ping it answers. It stays within
 * the `u32` the wire carries.
 */
export function nextPingId(): number {
  lastPing = (lastPing % 0xffff_ffff) + 1
  return lastPing
}
/** The page's closed sockets, each by what connects it at once instead of after its wait. */
const waiting = new Set<() => void>()

/** Reports why the page replaced its socket named `name`, for a developer; the user sees what the page shows. */
function reportReplaced(name: string, why: string): void {
  reportError(t('socketReplaced'), `The ${name} ${why}`)
}

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

/**
 * Opens a WebSocket to `url`, as every socket of the page opens: one whose
 * opening handshake has not completed within `HANDSHAKE_MS` cannot be made,
 * and is closed, which the web browser reports to its owner as it reports a
 * refused one, with its `close` event. An open socket that the backend
 * closes with a code of its own, or whose connection breaks, is reported
 * under `name`, such as "conversation socket"; one the page closes is not.
 */
export function openSocket(url: string | URL, name: string): WebSocket {
  const socket = new WebSocket(url)
  let opened = false
  let closedByPage = false
  // The close event does not say who started the close: a page's close of
  // a broken socket ends with 1006 as the break itself does. So the socket
  // notes the page's own close, whoever of its owners calls it.
  const close = socket.close.bind(socket)
  socket.close = (code?: number, reason?: string) => {
    closedByPage = true
    close(code, reason)
  }
  /** Whether the page's return abandoned it while it was opening. */
  let abandoned = false
  const settled = () => {
    clearTimeout(timer)
    opening.delete(socket)
    socket.removeEventListener('open', settled)
    socket.removeEventListener('close', settled)
  }
  const timer = setTimeout(() => {
    settled()
    // Its owner may have closed it meanwhile without its close coming yet.
    if (socket.readyState === WebSocket.CONNECTING) {
      socket.close()
    }
  }, HANDSHAKE_MS)
  opening.set(socket, {
    since: absences,
    abandon: () => {
      settled()
      abandoned = true
      reportReplaced(name, 'was still opening when the page returned')
      socket.close()
    },
  })
  socket.addEventListener('open', () => {
    opened = true
  }, { once: true })
  socket.addEventListener('open', settled)
  socket.addEventListener('close', settled)
  socket.addEventListener('close', (event: CloseEvent) => {
    // A socket that never opened is one more attempt that failed; 1000 is
    // a normal end, which its owner does not replace.
    if (opened && !closedByPage && event.code !== 1000) {
      reportReplaced(name, `closed with ${event.code}${event.reason ? ` ${event.reason}` : ''}`)
    }
    // Its owners start their wait as the close reaches them, which the
    // browser delivers after the return's own sweep of the waits: so it
    // connects again in the next task, not after the wait.
    if (abandoned) {
      setTimeout(connectWaiting, 0)
    }
  }, { once: true })
  return socket
}

/**
 * What sends the return's ping on `socket`, for the watch of a socket whose
 * far end is the backend, which answers every page socket's ping with
 * `pong` (`web-application.md` § Liveness and reconnection).
 */
export function pingOn(socket: WebSocket): (id: number) => void {
  return (id) => {
    if (socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ ping: id } satisfies PagePing))
    }
  }
}

/**
 * Calls `then` once the page has run the tasks that wait already: the
 * messages a browser held for a frozen tab and delivers once it runs again
 * among them, a heartbeat that shows a socket alive included. The browser
 * says so by calling the page idle, which it does only once nothing is
 * queued; where it cannot, as in Safari and in a test, `then` follows the
 * tasks queued before it.
 */
function afterQueuedTasks(then: () => void): void {
  if (defaultWindow && 'requestIdleCallback' in defaultWindow) {
    defaultWindow.requestIdleCallback(() => then())
    return
  }
  setTimeout(then, 0)
}

/** A watch over one socket's silence. */
export interface SilenceWatch {
  /** The socket brought a message, a heartbeat included: its silence starts again. */
  heard(): void
  /** The socket brought the far end's `pong` to the ping `id`. */
  ponged(id: number): void
  /** Ends the watch; nothing is called after it. */
  stop(): void
}

/**
 * Watches the socket named `name`, such as "conversation socket", from now
 * on and calls `broken` once when it has brought nothing for `SILENCE_MS`,
 * counted from now or from the last `heard`: when the watch's timer ends, or
 * at the page's return when the clock shows that much silence while the
 * timer slept. Either judges only once the page has read the messages
 * waiting for it, so a heartbeat a frozen tab held keeps its socket; a
 * judgement that finds less silence counts on from the last message by the
 * clock. With `ping`, which sends the far end a ping it answers with a
 * `pong` of the same id, the return then asks the socket whether it is
 * alive, as what it read may have waited in the browser since before the
 * page went away, and calls `broken` when the `pong` has not come within
 * `PONG_MS`. Each break is reported under `name`. The socket's owner stops
 * the watch when it lets the socket go, whatever the reason.
 */
export function watchSilence(name: string, broken: () => void, ping?: (id: number) => void): SilenceWatch {
  // The wall clock, which goes on while the machine sleeps.
  let heardAt = Date.now()
  let timer: ReturnType<typeof setTimeout> | null = null
  /** The return's ping waiting for its pong: its id, its deadline, and what the return waits on. */
  let asked: { id: number; deadline: ReturnType<typeof setTimeout>; judged: () => void } | null = null
  const clear = () => {
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = null
  }
  /** Ends the ping's wait: it was answered, or the watch ended. */
  const settle = () => {
    if (asked === null) {
      return
    }
    clearTimeout(asked.deadline)
    asked.judged()
    asked = null
  }
  const stop = () => {
    clear()
    settle()
    watched.delete(watch)
  }
  const lose = (why: Break) => {
    stop()
    reportReplaced(name, BREAK_WORDS[why])
    broken()
  }
  /** Breaks the socket for `why` when it has been silent long enough by the clock, else watches the rest. */
  const judge = (why: Break) => {
    // A watch that broke or stopped meanwhile stays so.
    if (!watched.has(watch)) {
      return
    }
    const silent = Date.now() - heardAt
    if (silent >= SILENCE_MS) {
      lose(why)
      return
    }
    arm(SILENCE_MS - silent)
  }
  const arm = (ms: number) => {
    clear()
    timer = setTimeout(() => {
      timer = null
      // A tab the browser froze runs its timers that came due as it runs
      // again, before or after the messages it held.
      afterQueuedTasks(() => judge('silence'))
    }, ms)
  }
  /** Asks the socket whether it is alive, and calls `judged` once it is judged. */
  const probe = (send: (id: number) => void, judged: () => void) => {
    // A return during a ping's wait asks again: only the latest pong counts.
    settle()
    const id = nextPingId()
    asked = {
      id,
      // As the silence timer's, this judgement waits for the messages
      // already queued, the pong among them.
      deadline: setTimeout(() => afterQueuedTasks(() => {
        if (asked?.id !== id) {
          return
        }
        lose('probe')
        // The socket's owner started its wait as the break reached it; the
        // return does not wait for it.
        connectWaiting()
        setTimeout(connectWaiting, 0)
      }), PONG_MS),
      judged,
    }
    send(id)
  }
  const watch = (judged: () => void) => {
    judge('return')
    if (ping === undefined || !watched.has(watch)) {
      judged()
      return
    }
    probe(ping, judged)
  }
  arm(SILENCE_MS)
  watched.add(watch)
  return {
    heard() {
      // A watch that broke or stopped stays so.
      if (!watched.has(watch)) {
        return
      }
      heardAt = Date.now()
      arm(SILENCE_MS)
    },
    ponged(id) {
      if (asked?.id === id) {
        settle()
      }
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

/** The page's return check that has not ended yet, if any, and the absence it is the return from. */
let returnCheck: { done: Promise<void>; absence: number } | null = null

/**
 * The page went away: it was hidden, frozen or put in the back-forward
 * cache, or went offline. A socket still opening then is abandoned at the
 * return, as its handshake belongs to a connection the system may have cut.
 */
export function pageWentAway(): void {
  absences += 1
}

/**
 * The page came back: it became visible again, the browser showed it from
 * its back-forward cache, a frozen page resumed, or it came back online.
 * Once it has read what its sockets hold, each socket still opening from
 * before the page went away is abandoned and opened again, each open socket
 * that has brought nothing for `SILENCE_MS` by the clock is broken at once,
 * and each other one watched for the rest of that time by the clock and,
 * where its far end can answer, asked for a `pong`, without which it is
 * broken after `PONG_MS`. Then each closed socket connects without waiting
 * for the rest of its wait, one just broken among them when its owner
 * starts the wait as the break reaches it. An owner that learns of the
 * break through a promise, as a conversation that still opens does, starts
 * its wait once the promises due have run, so the closed sockets are swept
 * once more when the task the check ran in has ended. Resolves once every
 * open socket was judged alive or broken. A return from a later absence
 * checks again: what the earlier check asked was answered, if at all,
 * before the page went away again, and says nothing of the sockets now.
 */
export function pageReturned(): Promise<void> {
  // One check serves every sign of a return from the same absence that
  // comes before it ends.
  if (returnCheck !== null && returnCheck.absence === absences) {
    return returnCheck.done
  }
  const absence = absences
  const done = new Promise<void>((resolve) => {
    afterQueuedTasks(() => {
      for (const [socket, { since, abandon }] of [...opening]) {
        if (since < absences && socket.readyState === WebSocket.CONNECTING) {
          abandon()
        }
      }
      // One more than the sockets to judge, which the check itself ends.
      let unjudged = watched.size + 1
      const judged = () => {
        unjudged -= 1
        if (unjudged === 0) {
          if (returnCheck?.done === done) {
            returnCheck = null
          }
          resolve()
        }
      }
      for (const check of [...watched]) {
        check(judged)
      }
      connectWaiting()
      // Left to run: it only connects the sockets that wait by then.
      setTimeout(connectWaiting, 0)
      judged()
    })
  })
  returnCheck = { done, absence }
  return done
}

/**
 * The page's return check while it is pending, else null. A request made as
 * the page returns waits for it, so that it goes on a socket the check found
 * alive or on the new one, never on one the check is about to replace
 * (`web-application.md` § A request the drop interrupts).
 */
export function pendingReturnCheck(): Promise<void> | null {
  return returnCheck?.done ?? null
}

/**
 * Each socket that waits to connect again connects now: the page's path to
 * a Host changed, as when a direct channel opened or failed
 * (`direct-channel.md` § Choosing the path), and a view its owner just
 * ended for the move connects on the new path; or the page reaches the
 * backend again after it was away, which the synchronization channel
 * learns first (`web-application.md` § Liveness and reconnection).
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

const online = ref(defaultNavigator?.onLine ?? true)
/** Running while the browser's word that it is offline has not held long enough to count. */
let offlineHold: ReturnType<typeof setTimeout> | null = null

/**
 * Whether the browser reports a network, as the page counts it: its word
 * that it is offline counts once it has held for `OFFLINE_HOLD_MS`. The
 * connection banner says so while it does not (`web-application.md` § A
 * page of another build), and the direct channel tries again when it
 * returns (`direct-channel.md` § Choosing the path).
 */
export const browserOnline = readonly(online)

/**
 * The browser said it went offline. Once that has held for a second, the
 * page is offline, and says so; its sockets stay as they are.
 */
export function browserWentOffline(): void {
  if (offlineHold !== null || !online.value) {
    return
  }
  offlineHold = setTimeout(() => {
    offlineHold = null
    online.value = false
    pageWentAway()
  }, OFFLINE_HOLD_MS)
}

/**
 * The browser said it is online. An offline that had not held yet was a
 * moment the page lets pass; after one that held, the page has returned.
 */
export function browserCameOnline(): void {
  if (offlineHold !== null) {
    clearTimeout(offlineHold)
    offlineHold = null
    return
  }
  if (online.value) {
    return
  }
  online.value = true
  void pageReturned()
}

/**
 * Whether the page is visible or hidden, for the page's lifetime: a view
 * nobody can see lets go of what it watches. Without a document, as in a
 * test, the page counts as visible.
 */
export const pageVisibility = readonly(useDocumentVisibility())

/**
 * Follows the page's going away and its return in `window` and `document`,
 * until the answer is called: hidden and visible again, `pagehide` and a
 * `pageshow` from the back-forward cache, `freeze` and `resume`, offline
 * and online.
 */
export function followPage(window: Pick<Window, 'addEventListener' | 'removeEventListener'>, document: Pick<Document, 'visibilityState' | 'addEventListener' | 'removeEventListener'>): () => void {
  const stops = [
    useEventListener(document, 'visibilitychange', () => {
      if (document.visibilityState === 'visible') {
        void pageReturned()
      } else {
        pageWentAway()
      }
    }),
    useEventListener(document, 'freeze', pageWentAway),
    useEventListener(document, 'resume', () => void pageReturned()),
    useEventListener(window, 'pagehide', pageWentAway),
    useEventListener(window, 'pageshow', (event: PageTransitionEvent) => {
      if (event.persisted) {
        void pageReturned()
      }
    }),
    useEventListener(window, 'offline', browserWentOffline),
    useEventListener(window, 'online', browserCameOnline),
  ]
  return () => {
    for (const stop of stops) {
      stop()
    }
  }
}

// For the page's lifetime; without a document, as in a test, there is no
// return to follow, and the test calls these in the browser's place.
if (defaultWindow && defaultDocument) {
  followPage(defaultWindow, defaultDocument)
}
