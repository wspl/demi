// The network primitives as the runner uses them: the backend WebSocket and
// the local relay socket. Byte streams over the handles come from `stdio.ts`.
import * as fs from 'tinyjs:fs'
import * as net from 'tinyjs:net'
import { readHandle } from './stdio'

export interface WebSocketLink {
  send(frame: Uint8Array): Promise<void>
  /** The next frame, or `null` once the peer closed. */
  receive(): Promise<Uint8Array | null>
  close(code?: number): Promise<void>
}

/** Connects the one outbound WebSocket; proxies from the environment apply. */
export async function connectWebSocket(url: string, headers?: Record<string, string>): Promise<WebSocketLink> {
  // An explicit `undefined` is not an absent optional to the primitive.
  const ws = headers ? await net.wsConnect(url, { headers }) : await net.wsConnect(url)
  let closed = false
  return {
    send: (frame) => net.wsSend(ws, frame),
    receive: () => net.wsRecv(ws),
    close: async (code) => {
      if (closed) return
      closed = true
      await net.wsClose(ws, code)
    },
  }
}

/** A connected stream socket: bytes in, bytes out. */
export interface StreamSocket {
  input: AsyncIterable<Uint8Array>
  write(data: Uint8Array): Promise<void>
  close(): void
}

function socketOf(fd: number): StreamSocket {
  let open = true
  return {
    input: readHandle(fd, false),
    write: (data) => fs.write(fd, data),
    close: () => {
      if (!open) return
      open = false
      fs.close(fd)
    },
  }
}

export interface UnixListener {
  accept(): Promise<StreamSocket>
  close(): void
}

/** Listens on a Unix domain socket created with `mode`. */
export async function listenUnix(path: string, mode: number): Promise<UnixListener> {
  const listener = await net.udsListen(path, { mode })
  return {
    accept: async () => socketOf(await net.accept(listener)),
    close: () => net.close(listener),
  }
}

export async function connectUnix(path: string): Promise<StreamSocket> {
  return socketOf(await net.udsConnect(path))
}
