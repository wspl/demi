import { expect, test } from 'bun:test'
import { Hono } from 'hono'
import { collectBytes, delay } from '@demicodes/utils'
import { transferRoutes } from '../http/transfers'
import { hashDeviceToken } from '../runner/claim-codes'
import { TransferBroker } from '../runner/transfers'
import type { ControlService } from '../storage/control'

// The broker pipes a source's PUT into a destination's GET (or into this
// process) with the bytes held only in flight; each transfer is single-use,
// authenticated per side by device token, and fails when a side never
// arrives or drops.

const tokens: Record<string, string> = { a: 'token-a', b: 'token-b' }
const control = {
  getDeviceByTokenHash: async (hash: string) => {
    const entry = Object.entries(tokens).find(([, token]) => hashDeviceToken(token) === hash)
    return entry ? ({ id: entry[0] } as never) : null
  },
} as unknown as ControlService

function serve(broker: TransferBroker) {
  const app = new Hono().route('/api/transfers', transferRoutes({ control, broker }))
  const server = Bun.serve({ port: 0, fetch: app.fetch })
  return { url: `http://localhost:${server.port}`, stop: () => server.stop(true) }
}

const bearer = (device: string) => ({ authorization: `Bearer ${tokens[device]}` })

test('runner to runner: PUT streams into GET, the PUT returns once the GET drained', async () => {
  const broker = new TransferBroker({ timeoutMs: 5_000 })
  const { url, stop } = serve(broker)
  const transfer = broker.open('a', { deviceId: 'b' })
  const payload = new Uint8Array(2 * 1024 * 1024).map((_, i) => i & 0xff)

  // The destination arrives first and waits for the source.
  const get = fetch(`${url}${transfer.url}`, { headers: bearer('b') })
  const put = fetch(`${url}${transfer.url}`, { method: 'PUT', body: payload, headers: bearer('a') })
  const response = await get
  expect(response.status).toBe(200)
  const body = new Uint8Array(await response.arrayBuffer())
  expect(Buffer.from(body).equals(Buffer.from(payload))).toBe(true)
  expect((await put).status).toBe(200)
  await transfer.done

  // Single use, and the wrong device on either side is a 404.
  expect((await fetch(`${url}${transfer.url}`, { headers: bearer('b') })).status).toBe(404)
  const second = broker.open('a', { deviceId: 'b' })
  expect((await fetch(`${url}${second.url}`, { headers: bearer('a') })).status).toBe(404)
  expect((await fetch(`${url}${second.url}`, { method: 'PUT', body: 'x', headers: bearer('b') })).status).toBe(404)
  expect((await fetch(`${url}${second.url}`, { method: 'PUT', body: 'x' })).status).toBe(401)
  broker.fail(second.id, 'test over')
  await expect(second.done).rejects.toThrow('test over')
  stop()
})

test('into this process: the consumer receives the PUT body in order; a missing side times out', async () => {
  const broker = new TransferBroker({ timeoutMs: 200 })
  const { url, stop } = serve(broker)
  const chunks: Uint8Array[] = []
  const transfer = broker.open('a', { consume: async (chunk) => void chunks.push(chunk) })
  const put = await fetch(`${url}${transfer.url}`, { method: 'PUT', body: 'hello, world', headers: bearer('a') })
  expect(put.status).toBe(200)
  await transfer.done
  expect(new TextDecoder().decode(await collectBytes((async function* () { yield* chunks })()))).toBe('hello, world')

  const lonely = broker.open('a', { deviceId: 'b' })
  await expect(lonely.done).rejects.toThrow('never arrived')
  expect((await fetch(`${url}${lonely.url}`, { method: 'PUT', body: 'late', headers: bearer('a') })).status).toBe(404)

  // A device dropping fails what it was party to, on both sides.
  const dropped = broker.open('a', { deviceId: 'b' })
  const waitingGet = fetch(`${url}${dropped.url}`, { headers: bearer('b') })
  await delay(50)
  broker.deviceGone('a')
  await expect(dropped.done).rejects.toThrow('disconnected')
  expect((await waitingGet).status).toBe(409)
  stop()
})
