import { afterEach, expect, test } from 'bun:test'
import { FileBrowserError } from '@demicodes/web-ui/files/types'
import { fileReads, rawFileContents } from './files'

// A raw route as the file views load it: URLs the web browser fetches itself, and
// HEAD answers read as file descriptions.

const originalFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = originalFetch
})

/** Answers every request with `response`, noting each one in `requests`. */
function answer(response: Response, requests: Array<{ url: string; method?: string }> = []): void {
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL, init?: RequestInit) => {
    requests.push({ url: String(input), method: init?.method })
    return response
  }, { preconnect: originalFetch.preconnect })
}

const contents = rawFileContents('/conversations/c1/fs/raw')

test('a URL names the file, and a version and download only when asked for', () => {
  expect(contents.url('/work/a b.png')).toBe('/api/conversations/c1/fs/raw?path=%2Fwork%2Fa+b.png')
  expect(contents.url('/work/a.png', { version: 'W/"1-2"', download: true }))
    .toBe('/api/conversations/c1/fs/raw?path=%2Fwork%2Fa.png&version=W%2F%221-2%22&download=true')
})

test("a HEAD answer describes the file: its size, time and version, with the Host's number before it read", async () => {
  const requests: Array<{ url: string; method?: string }> = []
  answer(new Response(null, {
    headers: {
      'content-length': '15822',
      'last-modified': 'Sat, 12 Sep 2026 09:30:00 GMT',
      etag: 'W/"3dce-19"',
      'x-demi-read-at': 'run.1.40',
    },
  }), requests)
  expect(await contents.describe('/work/a.png')).toEqual({
    value: { size: 15822, modifiedAt: '2026-09-12T09:30:00.000Z', version: 'W/"3dce-19"' },
    readAt: { epoch: 'run.1', number: 40 },
  })
  expect(requests).toEqual([{ url: '/api/conversations/c1/fs/raw?path=%2Fwork%2Fa.png', method: 'HEAD' }])

  // The committed route knows neither the time nor a version.
  answer(new Response(null, { headers: { 'content-length': '124' } }))
  expect(await contents.describe('dist/app.zip')).toEqual({ value: { size: 124, modifiedAt: null, version: null }, readAt: null })
})

test('a failed HEAD says what it can by its status, and malformed headers are refused', async () => {
  // A file too large is the Host's answer, with its version and number.
  answer(new Response(null, { status: 413, headers: { etag: 'W/"9"', 'x-demi-read-at': 'run.41' } }))
  const tooLarge = await contents.describe('poster.png').catch((error: unknown) => error)
  expect(tooLarge).toBeInstanceOf(FileBrowserError)
  expect(tooLarge).toMatchObject({ kind: 'too-large', version: 'W/"9"', readAt: { epoch: 'run', number: 41 } })
  answer(new Response(null, { status: 404 }))
  expect(await contents.describe('missing.png').catch((error: unknown) => error)).toMatchObject({ kind: 'not-found' })

  answer(new Response(null, { headers: { 'content-length': '5', 'last-modified': 'yesterday' } }))
  await expect(contents.describe('a.png')).rejects.toThrow('Invalid server response: Expected an HTTP date')
  answer(new Response(null, { headers: { 'content-length': 'many' } }))
  await expect(contents.describe('a.png')).rejects.toThrow('Invalid server response')
})

test('a failed read says what happened in the page\'s words, never the backend\'s internals', async () => {
  const reads = fileReads({ directory: '/conversations/c1/fs', text: '/conversations/c1/fs/file' }, null)
  const failure = (code: string, message: string) => new Response(JSON.stringify({ requestId: 'req_test000000000001', code, message }), { status: 503 })
  answer(failure('cloud_unavailable', 'Cloud runner reconnect timeout'))
  expect(await reads.readText!('/work/README.md', null).catch((error: unknown) => error))
    .toMatchObject({ kind: 'other', message: 'The Cloud isn’t available right now.' })
  answer(new Response(JSON.stringify({ requestId: 'req_test000000000001', code: 'device_offline', message: 'runner 4f2 has no connection' }), { status: 409 }))
  expect(await reads.list('/work').catch((error: unknown) => error))
    .toMatchObject({ kind: 'offline', message: 'The device is offline.' })
})

test('the start of a large file is one byte range, read up to the length asked for', async () => {
  const requests: Array<{ url: string; method?: string }> = []
  const ranges: Array<string | null> = []
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL, init?: RequestInit) => {
    requests.push({ url: String(input), method: init?.method })
    ranges.push(new Headers(init?.headers).get('range'))
    return new Response(new Uint8Array([104, 105, 33]), { status: 206 })
  }, { preconnect: originalFetch.preconnect })
  const bytes = await contents.readStart!('/work/big.log', 2, { version: 'W/"9"' })
  expect([...bytes]).toEqual([104, 105])
  expect(ranges).toEqual(['bytes=0-1'])
  expect(requests[0]?.url).toBe('/api/conversations/c1/fs/raw?path=%2Fwork%2Fbig.log&version=W%2F%229%22')
})

// Planted defect it catches: a listing that goes alone though others are asked for in the same
// turn (a deep file's folders, one round trip each), or one refused folder failing its neighbours.
test('the folders asked for together are listed in one request, each answering as it would alone', async () => {
  const requests: Array<{ url: string; method: string; body: unknown }> = []
  const listed = (path: string) => ({ type: 'listed', path, home: null, entries: [{ name: 'x', isDirectory: false, size: 3, modifiedAt: '2026-01-01T00:00:00.000Z' }], readAt: 'run.7' })
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL, init?: RequestInit) => {
    requests.push({ url: String(input), method: init?.method ?? 'GET', body: init?.body === undefined ? null : JSON.parse(String(init.body)) })
    if (init?.method !== 'POST')
      return new Response(JSON.stringify({ path: '/w', home: null, entries: [] }), { status: 200 })
    const refused = { type: 'refused', path: '/w/secret', status: 403, error: { requestId: 'req_test000000000001', code: 'host_operation_failed', message: 'Permission denied' } }
    return new Response(JSON.stringify({ lists: [listed('/w'), refused, listed('/w/a')] }), { status: 200 })
  }, { preconnect: originalFetch.preconnect })
  const reads = fileReads({ directory: '/conversations/c1/fs', lists: '/conversations/c1/fs/lists' }, null)

  const [first, second, third] = await Promise.allSettled([reads.list('/w'), reads.list('/w/secret'), reads.list('/w/a')])
  expect(requests).toEqual([{ url: '/api/conversations/c1/fs/lists', method: 'POST', body: { paths: ['/w', '/w/secret', '/w/a'] } }])
  expect(first).toMatchObject({ status: 'fulfilled', value: { value: [{ name: 'x' }], readAt: { epoch: 'run', number: 7 } } })
  expect(second).toMatchObject({ status: 'rejected', reason: { kind: 'permission' } })
  expect(third.status).toBe('fulfilled')

  // One folder is listed as it always was.
  requests.length = 0
  await reads.list('/w')
  expect(requests).toEqual([{ url: '/api/conversations/c1/fs?path=%2Fw', method: 'GET', body: null }])
})
