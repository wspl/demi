/**
 * The page's connection to the backend (`@demicodes/conversation-client`'s
 * `PageConnection`), as a web page runs it: a browser's WebSocket to
 * `/api/connect`, the page's liveness (`liveness.ts`): the backend's
 * heartbeat after 30 silent seconds, the 75-second silence and the ping at
 * the page's return, the page's waits before it connects again, and the
 * page's log. Where the connection stands is two refs, which the page shows.
 */
import { readonly, ref } from 'vue'
import { PageConnection, type Lane, type LaneTarget } from '@demicodes/conversation-client'
import { pageLog } from '../infra/log'
import { openSocket, reconnectNow, waitForConnection, watchSilence } from './liveness'

export { Lane, type LaneEventMap, type LaneRefusal, type LaneTarget } from '@demicodes/conversation-client'

/** What the page tells the connection once, as it starts. */
export interface PageConnectionOptions {
  /** The connection's address, `/api/connect`, as the page reaches the backend. */
  url: () => string
  /**
   * The connection could not be made. The web browser does not say why: an
   * ended session answers its upgrade with 401, which a request learns.
   */
  failedToOpen: () => void
}

let options: PageConnectionOptions | null = null

/** Connections in a row that were lost or could not be made before one counted as connected. */
const failures = ref(0)
/** The connection closed with 1001 `backend_closing`, and none counted as connected since. */
const restarting = ref(false)
/** Connections in a row that were lost or could not be made, for where the page's connection stands. */
export const connectionFailures = readonly(failures)
/** Whether the backend said it restarts, for where the page's connection stands. */
export const backendRestarting = readonly(restarting)

/** The page connection, which the page's owners of lanes share. */
export const pageConnection = new PageConnection({
  open: () => {
    const url = new URL(options!.url(), globalThis.location?.href)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    return openSocket(url, 'page connection')
  },
  watch: (broken, ping) => watchSilence('page connection', broken, ping),
  wait: waitForConnection,
  // Every owner that waits to connect connects at once.
  reached: reconnectNow,
  failedToOpen: () => options?.failedToOpen(),
  changed: (standing) => {
    failures.value = standing.failures
    restarting.value = standing.restarting
  },
  log: pageLog,
})

/** Starts the connection with `given`; a lane asked for before waits for it. */
export function configurePageConnection(given: PageConnectionOptions): void {
  options = given
  pageConnection.start()
}

/** Opens a lane that carries what `target` names, logged as `name`, such as "conversation lane". */
export function openLane(target: LaneTarget, name: string): Lane {
  return pageConnection.openLane(target, name)
}
