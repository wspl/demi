/**
 * A plugin's user stream over its lane of the page connection, or over a
 * view link's own socket, which speak alike: bytes sent before it opens wait
 * for it, and the close reason ends the stream. The plugin's bytes are
 * binary messages, and the stream's own word on whether it is watched a text
 * message.
 */
import type { StreamWatched } from '@demicodes/protocol'
import type { OpenUserStream, StreamBytes } from '../plugins/streams'
import { Lane, openLane } from './page-connection'

/** The stream `stream` of the conversation `conversation`. */
export function userStreamOf(conversation: string, stream: string): OpenUserStream {
  return streamOver(
    () => openLane({ kind: 'user_stream', conversation, stream }, 'user stream'),
    (lane) => lane.refusal?.error.message,
  )
}

/**
 * The browser stream of the view link `token`, on its own socket
 * (`crates/backend-http/src/browser_views.rs`), which a page without a
 * session opens.
 */
export function viewStreamOf(token: string): OpenUserStream {
  return streamOver(() => {
    const address = new URL(`/api/views/${encodeURIComponent(token)}/stream`, location.href)
    address.protocol = address.protocol === 'https:' ? 'wss:' : 'ws:'
    return new WebSocket(address)
  }, () => undefined)
}

/** A stream over what `open` makes, whose refusal's words `refused` reads once it closed without a reason. */
function streamOver<S extends Lane | WebSocket>(open: () => S, refused: (socket: S) => string | undefined): OpenUserStream {
  return (handlers) => {
    const lane = open()
    lane.binaryType = 'arraybuffer'
    const queued: StreamBytes[] = []
    let watched = true
    const sayWatched = () => lane.send(JSON.stringify({ watched } satisfies StreamWatched))
    lane.addEventListener('open', () => {
      // The backend takes a stream as watched until it says otherwise.
      if (!watched) {
        sayWatched()
      }
      for (const bytes of queued) {
        lane.send(bytes)
      }
      queued.length = 0
    })
    lane.addEventListener('message', (event) => {
      if (event.data instanceof ArrayBuffer) {
        handlers.data(new Uint8Array(event.data))
      }
    })
    lane.addEventListener('close', (event) => {
      const closed = event as CloseEvent
      handlers.closed(closed.reason || refused(lane) || `closed ${closed.code}`)
    })
    return {
      send(bytes) {
        if (lane.readyState === Lane.CONNECTING) {
          queued.push(bytes)
        } else if (lane.readyState === Lane.OPEN) {
          lane.send(bytes)
        }
      },
      watched(now) {
        if (now === watched) {
          return
        }
        watched = now
        if (lane.readyState === Lane.OPEN) {
          sayWatched()
        }
      },
      close() {
        queued.length = 0
        lane.close()
      },
    }
  }
}
