import { readFileSync, writeFileSync } from 'node:fs'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { createRunnerWire, type RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { deferred, waitFor } from '@demicodes/utils'
import { startTinyjsRunner } from '../testing'

// The runner's end of brokered transfers (`runner.md` § Transfers): told
// `transfer_receive`, it GETs the origin-relative URL with its device token
// into the file; told `transfer_send`, it PUTs the file; each ends in
// `transfer_done`. The fake backend below is the socket plus the two routes.

const wire = createRunnerWire(msgpackCodec)
const TOKEN = 'device-token-for-the-test'

test('transfer_receive writes the GET body to the file; transfer_send PUTs the file; both report transfer_done', async () => {
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-xfer-home-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-xfer-state-'))
  const payload = new Uint8Array(3 * 1024 * 1024)
  for (let i = 0; i < payload.length; i += 1) payload[i] = (i * 7) & 0xff

  const inbound: RunnerToBackendMessage[] = []
  const uploaded = deferred<Uint8Array>()
  const authorizations: string[] = []
  let socket: Bun.ServerWebSocket<unknown> | null = null
  const server = Bun.serve({
    port: 0,
    async fetch(request, bunServer) {
      const url = new URL(request.url)
      if (url.pathname === '/api/runner') return bunServer.upgrade(request) ? undefined : new Response('no', { status: 400 })
      authorizations.push(request.headers.get('authorization') ?? '')
      if (request.method === 'GET' && url.pathname === '/api/transfers/down') return new Response(payload)
      if (request.method === 'PUT' && url.pathname === '/api/transfers/up') {
        uploaded.resolve(new Uint8Array(await request.arrayBuffer()))
        return new Response('transferred')
      }
      // Drained before refusing, so the runner sees the status rather than a reset.
      await request.arrayBuffer()
      return new Response('no such transfer', { status: 404 })
    },
    websocket: {
      message(ws, data) {
        const message = wire.decodeRunnerToBackend(typeof data === 'string' ? new Uint8Array(0) : new Uint8Array(data))
        inbound.push(message)
        if (message.type === 'hello') {
          socket = ws
          ws.send(wire.encode({ type: 'claimed', deviceToken: TOKEN }))
        }
      },
    },
  })

  const runner = await startTinyjsRunner({ backendUrl: `http://localhost:${server.port}`, stateDir, home: runnerDir })
  await waitFor(() => runner.statuses.includes('online'), () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const send = (message: Parameters<typeof wire.encode>[0]) => socket!.send(wire.encode(message))

  const received = join(runnerDir, 'received.bin')
  send({ type: 'transfer_receive', transferId: 'r1', path: received, url: '/api/transfers/down' })
  await waitFor(() => inbound.some((m) => m.type === 'transfer_done' && m.transferId === 'r1'), () => runner.log.join('\n'), { timeoutMs: 10_000 })
  expect(inbound.find((m) => m.type === 'transfer_done' && m.transferId === 'r1')).toEqual({ type: 'transfer_done', transferId: 'r1', ok: true })
  expect(Buffer.from(readFileSync(received)).equals(Buffer.from(payload))).toBe(true)

  const source = join(runnerDir, 'source.bin')
  writeFileSync(source, payload.subarray(0, 1_000_001))
  send({ type: 'transfer_send', transferId: 's1', path: source, url: '/api/transfers/up' })
  await waitFor(() => inbound.some((m) => m.type === 'transfer_done' && m.transferId === 's1'), () => runner.log.join('\n'), { timeoutMs: 10_000 })
  expect(inbound.find((m) => m.type === 'transfer_done' && m.transferId === 's1')).toEqual({ type: 'transfer_done', transferId: 's1', ok: true })
  expect(Buffer.from(await uploaded.promise).equals(Buffer.from(payload.subarray(0, 1_000_001)))).toBe(true)
  expect(new Set(authorizations)).toEqual(new Set([`Bearer ${TOKEN}`]))

  // A refused exchange reports why; the file is never half-written as success.
  send({ type: 'transfer_send', transferId: 's2', path: source, url: '/api/transfers/gone' })
  await waitFor(() => inbound.some((m) => m.type === 'transfer_done' && m.transferId === 's2'), undefined, { timeoutMs: 10_000 })
  expect(inbound.find((m) => m.type === 'transfer_done' && m.transferId === 's2')).toEqual({ type: 'transfer_done', transferId: 's2', ok: false, error: 'transfer refused (404): no such transfer' })

  await runner.stop()
  server.stop(true)
}, 60_000)
