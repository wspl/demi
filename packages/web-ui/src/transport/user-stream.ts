/**
 * A plugin's user stream over its lane of the page connection: bytes sent
 * before the lane opens wait for it, and the close reason ends the stream.
 * The plugin's bytes are binary messages, and the stream's own word on
 * whether it is watched a text message.
 */
import type { StreamWatched } from '@demicodes/protocol'
import type { OpenUserStream, StreamBytes } from '../plugins/streams'
import { Lane, openLane } from './page-connection'

/** The stream `stream` of the conversation `conversation`. */
export function userStreamOf(conversation: string, stream: string): OpenUserStream {
  return (handlers) => {
    const lane = openLane({ kind: 'user_stream', conversation, stream }, 'user stream')
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
      handlers.closed(event.reason || lane.refusal?.error.message || `closed ${event.code}`)
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
