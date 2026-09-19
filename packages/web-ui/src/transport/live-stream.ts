/**
 * The product's live browser view: the conversation's `browser` user stream
 * (`web-api.md` § User streams), which carries the live protocol's bytes.
 */
import type { LiveBytes } from '../browser/frames'
import type { OpenLiveStream } from '../browser/session'

export function liveStreamAt(url: string): OpenLiveStream {
  return (handlers) => {
    const socket = new WebSocket(url)
    socket.binaryType = 'arraybuffer'
    const queued: LiveBytes[] = []
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
