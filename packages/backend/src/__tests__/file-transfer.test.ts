import { expect, test } from 'bun:test'
import { collectBytes, delay } from '@demicodes/utils'
import { TransferStalled, contentHeaders, pacedBody, rangeAnswer, requestChunks } from '../http/file-transfer'

// How a file's bytes are served (`web-api.md` § File text and working tree
// changes): the part a Range asks for, the headers that keep file content
// inert, and a body the browser paces.

test('a Range header asks for one part of the file, or for all of it', () => {
  expect(rangeAnswer(undefined, 100)).toEqual({
    status: 200, start: 0, length: 100, headers: { 'accept-ranges': 'bytes', 'content-length': '100' },
  })
  expect(rangeAnswer('bytes=10-19', 100)).toMatchObject({
    status: 206, start: 10, length: 10, headers: { 'content-range': 'bytes 10-19/100', 'content-length': '10' },
  })
  expect(rangeAnswer('bytes=90-', 100)).toMatchObject({ status: 206, start: 90, length: 10 })
  expect(rangeAnswer('bytes=-5', 100)).toMatchObject({ status: 206, start: 95, length: 5 })
  expect(rangeAnswer('bytes=-500', 100)).toMatchObject({ status: 206, start: 0, length: 100 })
  expect(rangeAnswer('bytes=50-5000', 100)).toMatchObject({ status: 206, start: 50, length: 50 })
  // Past the end, or an empty suffix: nothing can be sent.
  expect(rangeAnswer('bytes=100-', 100)).toEqual({
    status: 416, headers: { 'accept-ranges': 'bytes', 'content-range': 'bytes */100' },
  })
  expect(rangeAnswer('bytes=-0', 100).status).toBe(416)
  expect(rangeAnswer('bytes=0-', 0).status).toBe(416)
  // Several ranges, a reversed one, or another unit: the whole file.
  for (const header of ['bytes=0-1,5-6', 'bytes=9-3', 'items=0-1', 'bytes=-', 'bytes=a-b'])
    expect(rangeAnswer(header, 100).status).toBe(200)
})

test('media shows in place, images under a policy that keeps an SVG inert; the rest downloads', () => {
  expect(contentHeaders('image/svg+xml')).toEqual({
    'content-type': 'image/svg+xml',
    'content-security-policy': "default-src 'none'; style-src 'unsafe-inline'; sandbox",
    'x-content-type-options': 'nosniff',
  })
  // Chrome refuses audio opened directly under a policy; PDF viewers refuse a sandbox.
  expect(contentHeaders('audio/mpeg')).toEqual({ 'content-type': 'audio/mpeg', 'x-content-type-options': 'nosniff' })
  expect(contentHeaders('application/pdf')).toEqual({ 'content-type': 'application/pdf', 'x-content-type-options': 'nosniff' })
  const download = {
    'content-type': 'application/octet-stream',
    'x-content-type-options': 'nosniff',
  }
  expect(contentHeaders('text/html')).toEqual({ ...download, 'content-disposition': 'attachment' })
  expect(contentHeaders('text/markdown', { fileName: 'README.md' }))
    .toEqual({ ...download, 'content-disposition': 'attachment; filename="README.md"; filename*=UTF-8\'\'README.md' })
  expect(contentHeaders('image/png', { download: true, fileName: "图 (1)'s.png" })).toEqual({
    ...download,
    'content-disposition': "attachment; filename=\"_ (1)'s.png\"; filename*=UTF-8''%E5%9B%BE%20%281%29%27s.png",
  })
  expect(contentHeaders(null)).toEqual({ ...download, 'content-disposition': 'attachment' })
})

/** Pacing that nothing ends from outside, recording each ask to cut the connection short. */
function pacing(stallMs: number, signal = new AbortController().signal) {
  const cuts: number[] = []
  return { cuts, options: { stallMs, signal, cutShort: () => void cuts.push(Date.now()) } }
}

/** A source that counts what it produced and records being stopped. */
function source(chunks: number, fail?: Error) {
  const seen = { produced: 0, stopped: false }
  async function* bytes() {
    try {
      for (let index = 0; index < chunks; index += 1) {
        seen.produced += 1
        yield new Uint8Array([index])
      }
      if (fail)
        throw fail
    } finally {
      seen.stopped = true
    }
  }
  return { seen, bytes: bytes() }
}

test('a paced body passes the source through and is over when the source ends', async () => {
  const { bytes } = source(3)
  const { cuts, options } = pacing(1_000)
  const paced = pacedBody(bytes, options)
  expect(await collectBytes(paced.body)).toEqual(new Uint8Array([0, 1, 2]))
  await paced.finished
  expect(cuts).toEqual([])
})

test('a browser that stops taking bytes stops the source, and the body never ends complete', async () => {
  const { seen, bytes } = source(1_000)
  const { cuts, options } = pacing(50)
  const paced = pacedBody(bytes, options)
  const reader = paced.body.getReader()
  await reader.read()
  // Nothing asks for more: the stall stops the source and ends the transfer.
  await paced.finished
  expect(seen.stopped).toBe(true)
  expect(seen.produced).toBeLessThan(1_000)
  expect(cuts).toHaveLength(1)
  // A late read gets no clean end; it waits until the connection closes.
  const late = reader.read()
  expect(await Promise.race([late.then(() => 'answered'), delay(100).then(() => 'waiting')])).toBe('waiting')
  await reader.cancel()
})

test('a browser that leaves cancels the body, which stops the source', async () => {
  const { seen, bytes } = source(1_000)
  const { cuts, options } = pacing(1_000)
  const paced = pacedBody(bytes, options)
  const reader = paced.body.getReader()
  await reader.read()
  await reader.cancel()
  await paced.finished
  expect(seen.stopped).toBe(true)
  expect(cuts).toEqual([])
})

test('a failing source cuts the body short instead of ending it', async () => {
  const { bytes } = source(2, new Error('pipe failed'))
  const { cuts, options } = pacing(1_000)
  const paced = pacedBody(bytes, options)
  const reader = paced.body.getReader()
  expect((await reader.read()).value).toEqual(new Uint8Array([0]))
  expect((await reader.read()).value).toEqual(new Uint8Array([1]))
  const after = reader.read()
  await paced.finished
  expect(cuts).toHaveLength(1)
  expect(await Promise.race([after.then(() => 'answered'), delay(100).then(() => 'waiting')])).toBe('waiting')
  await reader.cancel()
})

test('ending the transfer cuts the body short at once, even while the browser is not reading', async () => {
  const { seen, bytes } = source(1_000)
  const transfer = new AbortController()
  const { cuts, options } = pacing(1_000, transfer.signal)
  const paced = pacedBody(bytes, options)
  const reader = paced.body.getReader()
  await reader.read()
  // No read is pending, as when the browser is behind on its buffer.
  transfer.abort(new Error('archived'))
  await paced.finished
  expect(seen.stopped).toBe(true)
  expect(cuts).toHaveLength(1)
  const after = reader.read()
  expect(await Promise.race([after.then(() => 'answered'), delay(100).then(() => 'waiting')])).toBe('waiting')
  await reader.cancel()
})

/** A request body that sends `chunks` and then, unless `close`, nothing more. */
function sentBody(chunks: number[][], close: boolean): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks)
        controller.enqueue(new Uint8Array(chunk))
      if (close)
        controller.close()
    },
  })
}

test('an upload body is read as the write asks, and time the write takes does not count against the browser', async () => {
  const signal = new AbortController().signal
  expect(await collectBytes(requestChunks(sentBody([[1, 2], [3]], true), { stallMs: 1_000, signal })))
    .toEqual(new Uint8Array([1, 2, 3]))
  const chunks = requestChunks(sentBody([[1], [2]], true), { stallMs: 30, signal })
  expect((await chunks.next()).value).toEqual(new Uint8Array([1]))
  // The Host is slow to take the first chunk: longer than the stall time.
  await delay(60)
  expect((await chunks.next()).value).toEqual(new Uint8Array([2]))
  expect((await chunks.next()).done).toBe(true)
  expect(await collectBytes(requestChunks(null, { stallMs: 30, signal }))).toEqual(new Uint8Array(0))
})

test('a browser that sends nothing while a chunk is asked for stalls the upload; ending the transfer fails it with its reason', async () => {
  const quiet = requestChunks(sentBody([[1]], false), { stallMs: 30, signal: new AbortController().signal })
  await quiet.next()
  await expect(quiet.next()).rejects.toBeInstanceOf(TransferStalled)

  const transfer = new AbortController()
  const ended = requestChunks(sentBody([[1]], false), { stallMs: 1_000, signal: transfer.signal })
  await ended.next()
  const reason = new Error('the conversation changed')
  const waiting = ended.next()
  transfer.abort(reason)
  await expect(waiting).rejects.toBe(reason)
})
