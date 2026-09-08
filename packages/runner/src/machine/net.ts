import { noop } from '@demicodes/utils'

export interface WebSocketLink {
  send(frame: Uint8Array): Promise<void>
  receive(): Promise<Uint8Array | null>
  close(code?: number): Promise<void>
}

export async function connectWebSocket(url: string, headers?: Record<string, string>): Promise<WebSocketLink> {
  const socket = new WebSocketStream(url, { headers })
  socket.closed.catch(noop)
  const opened = await socket.opened
  const reader = opened.readable.getReader()
  const writer = opened.writable.getWriter()
  writer.closed.catch(noop)
  return {
    send: (frame) => writer.write(frame),
    receive: async () => {
      const result = await reader.read()
      if (result.done) return null
      if (!(result.value instanceof Uint8Array)) throw new Error('runner WebSocket requires binary frames')
      return result.value
    },
    close: async (code) => { socket.close(code === undefined ? undefined : { closeCode: code }); await socket.closed.catch(noop) },
  }
}

export interface StreamSocket {
  input: AsyncIterable<Uint8Array>
  write(data: Uint8Array): Promise<void>
  close(): void
}

async function socketOf(socket: PipeSocket): Promise<StreamSocket> {
  socket.closed.catch(noop)
  const { readable, writable } = await socket.opened
  const writer = writable.getWriter()
  writer.closed.catch(noop)
  return { input: readable, write: (data) => writer.write(data), close: () => socket.close() }
}

export interface UnixListener {
  accept(): Promise<StreamSocket>
  close(): void
}

export async function listenUnix(path: string, mode: number): Promise<UnixListener> {
  const listener = await tjs.listen('pipe', path)
  listener.closed.catch(noop)
  try { await tjs.chmod(path, mode) }
  catch (error) { listener.close(); throw error }
  const reader = (await listener.opened).readable.getReader()
  return {
    accept: async () => {
      const result = await reader.read()
      if (result.done) throw new Error('Unix listener is closed')
      return socketOf(result.value)
    },
    close: () => listener.close(),
  }
}

export async function connectUnix(path: string): Promise<StreamSocket> {
  return socketOf(await tjs.connect('pipe', path))
}
