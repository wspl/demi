import { expect, test } from 'bun:test'
import { Hono } from 'hono'
import { bytesStream, collectBytes, delay, encodeUtf8 } from '@demicodes/utils'
import { pipeRoutes } from '../http/pipes'
import { hashDeviceToken } from '../runner/claim-codes'
import { PipeBroker } from '../runner/pipes'
import type { ControlService } from '../storage/control'

// The broker pipes a source's PUT into a sink's GET, or either end into
// this process, with the bytes held only in flight; an end may be named
// after the pipe is minted; each pipe is single-use, authenticated per
// device end, and fails when an end never arrives or a device drops.

const tokens: Record<string, string> = { a: 'token-a', b: 'token-b' }
const control = {
  getDeviceByTokenHash: async (hash: string) => {
    const entry = Object.entries(tokens).find(([, token]) => hashDeviceToken(token) === hash)
    return entry ? ({ id: entry[0] } as never) : null
  },
} as unknown as ControlService

function serve(broker: PipeBroker) {
  const app = new Hono().route('/api/pipes', pipeRoutes({ control, broker }))
  const server = Bun.serve({ port: 0, fetch: app.fetch })
  return { url: `http://localhost:${server.port}`, stop: () => server.stop(true) }
}

const bearer = (device: string) => ({ authorization: `Bearer ${tokens[device]}` })

test('device to device: PUT streams into GET, the PUT returns once the GET drained; ends named late', async () => {
  const broker = new PipeBroker({ timeoutMs: 5_000 })
  const { url, stop } = serve(broker)
  const payload = new Uint8Array(2 * 1024 * 1024).map((_, i) => i & 0xff)

  // The source is minted with the pipe, the sink named later — the sink arrives first and waits for the source.
  const pipe = broker.open({ deviceId: 'a' })
  pipe.sinkTo('b')
  expect(() => pipe.sinkTo('a')).toThrow('already fixed')
  const get = fetch(`${url}${pipe.url}`, { headers: bearer('b') })
  const put = fetch(`${url}${pipe.url}`, { method: 'PUT', body: payload, headers: bearer('a') })
  const response = await get
  expect(response.status).toBe(200)
  const body = new Uint8Array(await response.arrayBuffer())
  expect(Buffer.from(body).equals(Buffer.from(payload))).toBe(true)
  expect((await put).status).toBe(200)
  await pipe.done

  // Single use, and the wrong device on either side is a 404.
  expect((await fetch(`${url}${pipe.url}`, { headers: bearer('b') })).status).toBe(404)
  const second = broker.open({ deviceId: 'a' }, { deviceId: 'b' })
  expect((await fetch(`${url}${second.url}`, { headers: bearer('a') })).status).toBe(404)
  expect((await fetch(`${url}${second.url}`, { method: 'PUT', body: 'x', headers: bearer('b') })).status).toBe(404)
  expect((await fetch(`${url}${second.url}`, { method: 'PUT', body: 'x' })).status).toBe(401)
  broker.fail(second.id, 'test over')
  await expect(second.done).rejects.toThrow('test over')
  stop()
})

test('this process as the sink pulls the PUT body as it iterates; as the source it feeds the GET with backpressure', async () => {
  const broker = new PipeBroker({ timeoutMs: 5_000 })
  const { url, stop } = serve(broker)

  const inbound = broker.open({ deviceId: 'a' })
  const put = fetch(`${url}${inbound.url}`, { method: 'PUT', body: 'hello, world', headers: bearer('a') })
  expect(new TextDecoder().decode(await collectBytes(inbound.stream()))).toBe('hello, world')
  expect((await put).status).toBe(200)
  await inbound.done

  const outbound = broker.open(undefined, { deviceId: 'b' })
  const writer = outbound.writer()
  const wrote: string[] = []
  const producer = (async () => {
    for (const word of ['one ', 'two ', 'three']) {
      await writer.write(encodeUtf8(word))
      wrote.push(word)
    }
    writer.end()
  })()
  // Nothing is written until the sink pulls: the writer waits for the GET.
  await delay(50)
  expect(wrote).toEqual([])
  const response = await fetch(`${url}${outbound.url}`, { headers: bearer('b') })
  expect(await response.text()).toBe('one two three')
  await producer
  await outbound.done

  // A pipe that never leaves this process on either end still works, in order.
  const local = broker.open()
  const local_writer = local.writer()
  void (async () => {
    for await (const chunk of bytesStream(encodeUtf8('local'))) await local_writer.write(chunk)
    local_writer.end()
  })()
  expect(new TextDecoder().decode(await collectBytes(local.stream()))).toBe('local')
  stop()
})

test('a missing end times out; a device dropping fails what it was party to; a sink stopping early counts as drained', async () => {
  const broker = new PipeBroker({ timeoutMs: 200 })
  const { url, stop } = serve(broker)

  const lonely = broker.open({ deviceId: 'a' }, { deviceId: 'b' })
  await expect(lonely.done).rejects.toThrow('never arrived')
  expect((await fetch(`${url}${lonely.url}`, { method: 'PUT', body: 'late', headers: bearer('a') })).status).toBe(404)

  const dropped = broker.open({ deviceId: 'a' }, { deviceId: 'b' })
  const waitingGet = fetch(`${url}${dropped.url}`, { headers: bearer('b') })
  await delay(50)
  broker.deviceGone('a')
  await expect(dropped.done).rejects.toThrow('disconnected')
  expect((await waitingGet).status).toBe(409)

  // The sink takes one chunk and leaves: the source's PUT completes, as a closed pipe would let it.
  const early = broker.open({ deviceId: 'a' })
  const big = new Uint8Array(4 * 1024 * 1024)
  const put = fetch(`${url}${early.url}`, { method: 'PUT', body: big, headers: bearer('a') })
  for await (const chunk of early.stream()) {
    expect(chunk.byteLength).toBeGreaterThan(0)
    break
  }
  expect((await put).status).toBe(200)
  await early.done
  stop()
})

test('arrival timeout stops once both ends connect, even when the producer is quiet', async () => {
  const broker = new PipeBroker({ timeoutMs: 40 })
  const pipe = broker.open()
  const writer = pipe.writer()
  const iterator = pipe.stream()[Symbol.asyncIterator]()
  const first = iterator.next()
  await delay(80)
  await writer.write(encodeUtf8('late'))
  expect((await first).value).toEqual(encodeUtf8('late'))
  writer.end()
  expect((await iterator.next()).done).toBe(true)
  await pipe.done
  broker.close()
})

test('device ends connect once and remain alive after the arrival timeout', async () => {
  const broker = new PipeBroker({ timeoutMs: 40 })
  const pipe = broker.open({ deviceId: 'a' }, { deviceId: 'b' })
  let source!: ReadableStreamDefaultController<Uint8Array>
  const put = broker.put(pipe.id, 'a', new ReadableStream({ start(controller) { source = controller } }))
  const get = await broker.get(pipe.id, 'b')
  expect((await broker.get(pipe.id, 'b')).status).toBe(409)
  expect((await broker.put(pipe.id, 'a', new ReadableStream())).status).toBe(409)
  if (!('body' in get)) throw new Error('missing pipe body')
  const reader = get.body.getReader()
  const read = reader.read()
  await delay(80)
  source.enqueue(encodeUtf8('late'))
  source.close()
  expect((await read).value).toEqual(encodeUtf8('late'))
  expect((await reader.read()).done).toBe(true)
  expect((await put).status).toBe(200)
  await pipe.done
  broker.close()
})

test('failure interrupts an active device body and its pending read', async () => {
  const broker = new PipeBroker()
  const pipe = broker.open({ deviceId: 'a' }, { deviceId: 'b' })
  let cancelled = false
  const put = broker.put(pipe.id, 'a', new ReadableStream({ cancel() { cancelled = true } }))
  const get = await broker.get(pipe.id, 'b')
  if (!('body' in get)) throw new Error('missing pipe body')
  const read = get.body.getReader().read()
  broker.fail(pipe.id, 'cancelled')
  await expect(read).rejects.toThrow('cancelled')
  expect((await put).status).toBe(409)
  expect(cancelled).toBe(true)
  broker.close()
})

test('handing a quiet process source to a device requires that device to arrive', async () => {
  const broker = new PipeBroker({ timeoutMs: 40 })
  const pipe = broker.open(undefined, { deviceId: 'b' })
  pipe.writer()
  const get = broker.get(pipe.id, 'b')
  pipe.sourceFrom('a')
  await expect(pipe.done).rejects.toThrow('never arrived')
  expect((await get).status).toBe(409)
  broker.close()
})
