import { afterEach, expect, test } from 'bun:test'
import { FileBrowserError } from '@demicodes/web-ui/files/types'
import { rawFileContents } from './files'

// A raw route as the file views load it (`web-api.md` § File text and working
// tree changes): URLs the browser fetches itself, and HEAD answers read as
// file descriptions.

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

test('a HEAD answer describes the file: its size, time and version', async () => {
  const requests: Array<{ url: string; method?: string }> = []
  answer(new Response(null, {
    headers: { 'content-length': '15822', 'last-modified': 'Sat, 12 Sep 2026 09:30:00 GMT', etag: 'W/"3dce-19"' },
  }), requests)
  expect(await contents.describe('/work/a.png')).toEqual({
    size: 15822,
    modifiedAt: '2026-09-12T09:30:00.000Z',
    version: 'W/"3dce-19"',
  })
  expect(requests).toEqual([{ url: '/api/conversations/c1/fs/raw?path=%2Fwork%2Fa.png', method: 'HEAD' }])

  // The committed route knows neither the time nor a version.
  answer(new Response(null, { headers: { 'content-length': '124' } }))
  expect(await contents.describe('dist/app.zip')).toEqual({ size: 124, modifiedAt: null, version: null })
})

test('a failed HEAD says what it can by its status, and malformed headers are refused', async () => {
  answer(new Response(null, { status: 413 }))
  const tooLarge = await contents.describe('poster.png').catch((error: unknown) => error)
  expect(tooLarge).toBeInstanceOf(FileBrowserError)
  expect(tooLarge).toMatchObject({ kind: 'too-large' })
  answer(new Response(null, { status: 404 }))
  expect(await contents.describe('missing.png').catch((error: unknown) => error)).toMatchObject({ kind: 'not-found' })

  answer(new Response(null, { headers: { 'content-length': '5', 'last-modified': 'yesterday' } }))
  await expect(contents.describe('a.png')).rejects.toThrow('Invalid server response: Expected an HTTP date')
  answer(new Response(null, { headers: { 'content-length': 'many' } }))
  await expect(contents.describe('a.png')).rejects.toThrow('Invalid server response')
})
