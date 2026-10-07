// A local TCP echo server behind the network; about 0.5 s, most of it the
// latency the second test proves.
import { afterEach, expect, test } from 'bun:test'
import net from 'node:net'
import { Network } from './network'

const closers: (() => Promise<void>)[] = []

afterEach(async () => {
  for (const close of closers.splice(0)) {
    await close()
  }
})

/** An echo server on a free port, and the network in front of it. */
async function setUp(): Promise<Network> {
  const echo = net.createServer((socket) => socket.pipe(socket))
  await new Promise<void>((resolve) => echo.listen(0, '127.0.0.1', resolve))
  const address = echo.address()
  if (address === null || typeof address === 'string') {
    throw new Error('no port')
  }
  const network = await Network.listen(0, address.port)
  closers.push(async () => {
    await network.shutdown()
    await new Promise<void>((resolve) => echo.close(() => resolve()))
  })
  return network
}

/** A connection through the network that has sent `request` and had it echoed. */
async function connect(network: Network, request: string): Promise<net.Socket> {
  const socket = net.connect(network.port, '127.0.0.1')
  closers.push(async () => {
    socket.destroy()
  })
  await roundTrip(socket, request)
  return socket
}

function roundTrip(socket: net.Socket, text: string): Promise<void> {
  return new Promise((resolve) => {
    let received = ''
    const read = (chunk: Buffer) => {
      received += chunk.toString()
      if (received.length >= text.length) {
        socket.off('data', read)
        resolve()
      }
    }
    socket.on('data', read)
    socket.write(text)
  })
}

const SOCKET = 'GET /api/conversations/c-1/stream HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n'
const SYNC = 'GET /api/sync HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n'

test('a cut resets the connections whose requests match, and no other', async () => {
  const network = await setUp()
  const stream = await connect(network, SOCKET)
  const sync = await connect(network, SYNC)
  const ended = new Promise<string>((resolve) => stream.once('error', (error: NodeJS.ErrnoException) => resolve(error.code ?? '')))
  expect(network.cut('/\\/stream$/')).toEqual([expect.stringContaining('socket GET /api/conversations/c-1/stream')])
  // A reset, not a close: the page sees the socket fail without a close frame.
  expect(await ended).toBe('ECONNRESET')
  await roundTrip(sync, 'still there')
  expect(network.list()).toEqual([expect.stringContaining('/api/sync')])
})

test('latency delays the app\'s traffic with its backend by its round trip, and not the dev server\'s files', async () => {
  const network = await setUp()
  const api = await connect(network, SYNC)
  const module = await connect(network, 'GET /src/main.ts HTTP/1.1\r\nHost: x\r\n\r\n')
  network.set({ latencyMs: 200 })
  let began = performance.now()
  await roundTrip(module, 'GET /src/App.vue HTTP/1.1\r\nHost: x\r\n\r\n')
  expect(performance.now() - began).toBeLessThan(100)
  began = performance.now()
  await roundTrip(api, 'frame')
  expect(performance.now() - began).toBeGreaterThanOrEqual(195)
})
