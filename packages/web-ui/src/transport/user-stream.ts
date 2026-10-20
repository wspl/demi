/**
 * A plugin's user stream over the product's WebSocket at `url`: bytes sent
 * before the socket opens wait for it, and the close reason ends the stream.
 * The plugin's bytes are binary messages, and the stream's own word on
 * whether it is watched a text message.
 */
import type { StreamWatched } from '@demicodes/protocol'
import type { OpenUserStream, StreamBytes } from '../plugins/streams'
import { openSocket } from './liveness'

export function userStreamAt(url: string): OpenUserStream {
  return (handlers) => {
    const socket = openSocket(url, 'user stream')
    socket.binaryType = 'arraybuffer'
    const queued: StreamBytes[] = []
    let watched = true
    const sayWatched = () => socket.send(JSON.stringify({ watched } satisfies StreamWatched))
    socket.addEventListener('open', () => {
      // The backend takes a stream as watched until it says otherwise.
      if (!watched) {
        sayWatched()
      }
      for (const bytes of queued) {
        socket.send(bytes)
      }
      queued.length = 0
    })
    socket.addEventListener('message', (event: MessageEvent) => {
      if (event.data instanceof ArrayBuffer) {
        handlers.data(new Uint8Array(event.data))
      }
    })
    socket.addEventListener('close', (event: CloseEvent) => {
      handlers.closed(event.reason || `closed ${event.code}`)
    })
    return {
      send(bytes) {
        if (socket.readyState === WebSocket.CONNECTING) {
          queued.push(bytes)
        } else if (socket.readyState === WebSocket.OPEN) {
          socket.send(bytes)
        }
      },
      watched(now) {
        if (now === watched) {
          return
        }
        watched = now
        if (socket.readyState === WebSocket.OPEN) {
          sayWatched()
        }
      },
      close() {
        queued.length = 0
        socket.close()
      },
    }
  }
}
