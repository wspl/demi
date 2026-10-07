/**
 * A plugin's user stream over the product's WebSocket at `url`
 * (`web-api.md` § User streams): bytes sent before the socket opens wait
 * for it, and the close reason ends the stream.
 */
import type { OpenUserStream, StreamBytes } from '../plugins/streams'
import { openSocket } from './liveness'

export function userStreamAt(url: string): OpenUserStream {
  return (handlers) => {
    const socket = openSocket(url)
    socket.binaryType = 'arraybuffer'
    const queued: StreamBytes[] = []
    socket.addEventListener('open', () => {
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
      close() {
        queued.length = 0
        socket.close()
      },
    }
  }
}
