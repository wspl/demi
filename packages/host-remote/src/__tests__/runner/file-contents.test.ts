import { afterEach, expect, test } from 'bun:test'
import { mkdir, mkdtemp, readdir, readFile, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PipeBroker, RemoteHost, devicePipes } from '@demicodes/host-remote'
import { connectTestRunner, TEST_RUNNER_DEVICE } from '@demicodes/host-remote/testing'
import { MAX_MESSAGE_BYTES, type RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { memoryHostStore } from '@demicodes/shell/testing'
import { collectBytes, errorCode, waitFor } from '@demicodes/utils'

// File contents travel through pipes, never in a runner message
// (`runner.md` § File contents): any size, a byte range on request, the
// reader's departure stops the read, and a write lands whole or not at all.
// A message over the limit fails its own request and leaves the connection.

const MiB = 1024 * 1024
const cleanups: (() => Promise<void>)[] = []
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup()
})

async function connected() {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'demi-contents-')))
  cleanups.push(() => rm(dir, { recursive: true, force: true }))
  const pipes = new PipeBroker()
  const host = new RemoteHost({
    defaultCwd: dir,
    identity: { uid: 0, gid: 0, hostname: 'test', homeDir: dir },
    store: memoryHostStore(),
    pipes: devicePipes(pipes, TEST_RUNNER_DEVICE),
  })
  const inbound: RunnerToBackendMessage[] = []
  let disconnects = 0
  const connection = await connectTestRunner({
    home: dir,
    pipes,
    onHello: (send, hello) => host.attach(send, hello.runner.identity),
    onMessage: (message) => {
      inbound.push(message)
      host.handleMessage(message)
    },
    onClose: () => {
      disconnects += 1
      host.detach()
    },
  })
  cleanups.push(connection.close)
  return { dir, host, inbound, disconnects: () => disconnects }
}

/** Bytes that differ at every position, so a misplaced range shows. */
function pattern(size: number): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(size)
  for (let index = 0; index < size; index += 1)
    bytes[index] = (index * 31 + (index >> 8)) % 251
  return bytes
}

test('a file far over the message limit reads and writes whole, and the connection stays', async () => {
  const { dir, host, disconnects } = await connected()
  const size = 5 * MAX_MESSAGE_BYTES + 3
  const bytes = pattern(size)
  await writeFile(join(dir, 'large.bin'), bytes)
  expect(await host.fs.readFile(join(dir, 'large.bin'))).toEqual(bytes)
  await host.fs.writeFile(join(dir, 'copy/large.bin'), bytes, { createParents: true })
  expect(new Uint8Array(await readFile(join(dir, 'copy/large.bin')))).toEqual(bytes)
  expect(await readdir(join(dir, 'copy'))).toEqual(['large.bin'])
  expect(disconnects()).toBe(0)
  expect(host.online).toBe(true)
}, 60_000)

test('a stream reads one byte range, and a missing file rejects before any byte', async () => {
  const { dir, host } = await connected()
  const bytes = pattern(3 * MiB)
  await writeFile(join(dir, 'video.bin'), bytes)
  const range = async (offset: number, length?: number) => collectBytes(
    await host.fs.readStream(join(dir, 'video.bin'), { offset, length })
  )
  expect(await range(MiB + 7, 4096)).toEqual(bytes.subarray(MiB + 7, MiB + 7 + 4096))
  expect(await range(bytes.length - 10)).toEqual(bytes.subarray(bytes.length - 10))
  expect(await range(0, 0)).toEqual(new Uint8Array())
  const refused = await host.fs.readStream(join(dir, 'missing.bin')).then(() => null, (error: unknown) => error)
  expect(errorCode(refused)).toBe('ENOENT')
  const directory = await host.fs.readStream(dir).then(() => null, (error: unknown) => error)
  expect(errorCode(directory)).toBe('EISDIR')
}, 60_000)

test('a reader that leaves stops the runner read, and the Host keeps working', async () => {
  const { dir, host, inbound } = await connected()
  await writeFile(join(dir, 'long.bin'), pattern(64 * MiB))
  const controller = new AbortController()
  const stream = await host.fs.readStream(join(dir, 'long.bin'), { signal: controller.signal })
  let received = 0
  const reading = (async () => {
    for await (const chunk of stream) {
      received += chunk.byteLength
      if (received > MiB)
        controller.abort()
    }
  })()
  expect(await reading.then(() => null, (error: unknown) => error)).toBeInstanceOf(Error)
  // The runner's upload failed with the pipe: it reports the pipe's end.
  await waitFor(() => inbound.some((message) => message.type === 'pipe_done' && !message.ok))
  expect(received).toBeLessThan(64 * MiB)
  const early = await host.fs.readStream(join(dir, 'long.bin'))
  for await (const _chunk of early) break
  expect(await host.fs.exists(join(dir, 'long.bin'))).toBe(true)
}, 60_000)

test('a write that cannot land leaves the destination as it was and no temporary file', async () => {
  const { dir, host } = await connected()
  const missingParent = await host.fs.writeFile(join(dir, 'absent/file'), pattern(10))
    .then(() => null, (error: unknown) => error)
  expect(errorCode(missingParent)).toBe('ENOENT')
  await mkdir(join(dir, 'target'))
  await writeFile(join(dir, 'target/kept'), 'kept')
  const overDirectory = await host.fs.writeFile(join(dir, 'target'), pattern(2 * MiB))
    .then(() => null, (error: unknown) => error)
  expect(overDirectory).toBeInstanceOf(Error)
  expect(await readFile(join(dir, 'target/kept'), 'utf8')).toBe('kept')
  expect((await readdir(dir)).filter((name) => name.startsWith('.demi-write-'))).toEqual([])
  await writeFile(join(dir, 'replaced'), 'old')
  await host.fs.writeFile(join(dir, 'replaced'), new TextEncoder().encode('new'))
  expect(await readFile(join(dir, 'replaced'), 'utf8')).toBe('new')
}, 60_000)

test('a message over the limit fails its own request in either direction', async () => {
  const { dir, host, disconnects } = await connected()
  const request = await host.fs.stat(join(dir, 'x'.repeat(MAX_MESSAGE_BYTES)))
    .then(() => null, (error: unknown) => error)
  expect(errorCode(request)).toBe('too_large')
  // Long names make a listing outgrow the limit with a few thousand entries.
  const listing = join(dir, 'listing')
  await mkdir(listing)
  const name = 'n'.repeat(200)
  for (let index = 0; index <= MAX_MESSAGE_BYTES / 200; index += 1)
    await writeFile(join(listing, `${name}${index}`), '')
  const reply = await host.fs.readdir(listing).then(() => null, (error: unknown) => error)
  expect(errorCode(reply)).toBe('too_large')
  expect(await host.fs.exists(listing)).toBe(true)
  expect(disconnects()).toBe(0)
}, 120_000)
