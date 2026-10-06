import { expect, test } from 'bun:test'
import { IMAGE_CONTENT_POLICY } from '@demicodes/protocol'
import { rangeAnswer, rawAnswer, type RawRequest } from './raw'

// The service worker's answers to a raw request over the direct channel
// against the relay's route for the same file (`web-api.md` § File text
// and working tree changes): the same status, headers and part, from the
// relay's own cases (`RangeAnswer`, `content_headers`, `raw`).

const file = { size: 1000, version: 'W/"3e8-18f"', modifiedAt: '2026-09-21T14:13:20.123Z' }

function request(fields: Partial<RawRequest>): RawRequest {
  return { path: '/work/clip.mp4', version: null, download: false, range: null, ifNoneMatch: null, ...fields }
}

test('ranges are answered as the relay answers them', () => {
  expect(rangeAnswer(null, 1000)).toEqual({ kind: 'whole', size: 1000 })
  expect(rangeAnswer('bytes=0-99', 1000)).toEqual({ kind: 'part', start: 0, length: 100, size: 1000 })
  expect(rangeAnswer('bytes=900-', 1000)).toEqual({ kind: 'part', start: 900, length: 100, size: 1000 })
  expect(rangeAnswer('bytes=-100', 1000)).toEqual({ kind: 'part', start: 900, length: 100, size: 1000 })
  expect(rangeAnswer('bytes=-5000', 1000)).toEqual({ kind: 'part', start: 0, length: 1000, size: 1000 })
  expect(rangeAnswer('bytes=990-5000', 1000)).toEqual({ kind: 'part', start: 990, length: 10, size: 1000 })
  // Several ranges, a reversed one or one that does not parse: the whole file.
  expect(rangeAnswer('bytes=0-1,5-9', 1000)).toEqual({ kind: 'whole', size: 1000 })
  expect(rangeAnswer('bytes=9-5', 1000)).toEqual({ kind: 'whole', size: 1000 })
  expect(rangeAnswer('items=0-5', 1000)).toEqual({ kind: 'whole', size: 1000 })
  // Past the end, or an empty suffix: 416.
  expect(rangeAnswer('bytes=1000-', 1000)).toEqual({ kind: 'unsatisfiable', size: 1000 })
  expect(rangeAnswer('bytes=-0', 1000)).toEqual({ kind: 'unsatisfiable', size: 1000 })
  expect(rangeAnswer('bytes=0-', 0)).toEqual({ kind: 'unsatisfiable', size: 0 })
})

test('a video\'s part carries the relay\'s headers, its range and its version', () => {
  expect(rawAnswer(request({ range: 'bytes=100-199' }), file)).toEqual({
    status: 206,
    headers: {
      'cache-control': 'private, no-cache',
      vary: 'Cookie',
      'x-accel-buffering': 'no',
      'x-content-type-options': 'nosniff',
      'content-type': 'video/mp4',
      etag: file.version,
      'last-modified': 'Mon, 21 Sep 2026 14:13:20 GMT',
      'accept-ranges': 'bytes',
      'content-range': 'bytes 100-199/1000',
    },
    bytes: { offset: 100, length: 100 },
  })
  // A URL that names its version is kept for a year.
  const versioned = rawAnswer(request({ version: file.version }), file)
  expect(versioned.status).toBe(200)
  expect(versioned.headers['cache-control']).toBe('private, max-age=31536000, immutable')
  expect(versioned.bytes).toEqual({ offset: 0, length: 1000 })
})

test('an image is inert, anything else and a download is an attachment named after the file', () => {
  const image = rawAnswer(request({ path: '/work/logo.svg' }), file)
  expect(image.headers['content-type']).toBe('image/svg+xml')
  expect(image.headers['content-security-policy']).toBe(IMAGE_CONTENT_POLICY)
  expect(image.headers['content-disposition']).toBeUndefined()

  const page = rawAnswer(request({ path: '/work/index.html' }), file)
  expect(page.headers['content-type']).toBe('application/octet-stream')
  expect(page.headers['content-disposition']).toBe('attachment; filename="index.html"; filename*=UTF-8\'\'index.html')

  const download = rawAnswer(request({ path: 'C:\\work\\图 (1)\'s.png', download: true }), file)
  expect(download.headers['content-type']).toBe('application/octet-stream')
  expect(download.headers['content-disposition']).toBe(
    'attachment; filename="_ (1)\'s.png"; filename*=UTF-8\'\'%E5%9B%BE%20%281%29%27s.png',
  )
  expect(download.headers['content-security-policy']).toBeUndefined()
})

test('a version the page holds answers 304 and a range past the end 416, without bytes', () => {
  const held = rawAnswer(request({ ifNoneMatch: '"3e8-18f"' }), file)
  expect(held).toEqual({
    status: 304,
    headers: { 'cache-control': 'private, no-cache', vary: 'Cookie', 'x-accel-buffering': 'no', etag: file.version },
    bytes: null,
  })
  const past = rawAnswer(request({ range: 'bytes=2000-' }), file)
  expect(past.status).toBe(416)
  expect(past.headers['content-range']).toBe('bytes */1000')
  expect(past.bytes).toBeNull()
})
