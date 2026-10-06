import { afterEach, expect, test } from 'bun:test'
import { uploadAttachment } from './uploads'

// An upload from the composer (`web-api.md` § Uploads and media): the page
// names the file's SHA-256 first and sends its bytes only when the backend
// does not hold them, naming the same SHA-256 again.

const originalFetch = globalThis.fetch
const originalXhr = globalThis.XMLHttpRequest
afterEach(() => {
  globalThis.fetch = originalFetch
  globalThis.XMLHttpRequest = originalXhr
})

const file = new File(['# Notes\n'], 'notes.md', { type: 'text/markdown' })
/** The SHA-256 of the file's bytes, as `shasum -a 256` prints it. */
const sha256 = '365d0b84ae63c2afc293dedd2b00bdf0dc8d6ef70c9297d90f9e5682ab0d72ee'

function answerBody(): string {
  return JSON.stringify({
    attachment: {
      id: 'up-1',
      mediaType: 'text/markdown',
      sizeBytes: 8,
      sha256,
      createdAt: '2026-10-06T08:00:00.000Z',
      snippet: '# Notes\n',
    },
  })
}

interface Sent {
  method: string
  url: string
  body: unknown
}

/** Answers each fetch with `response`, noting each request in `sent`. */
function answerFetch(response: () => Response, sent: Sent[]): void {
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL, init?: RequestInit) => {
    sent.push({ method: init?.method ?? 'GET', url: String(input), body: init?.body })
    return response()
  }, { preconnect: originalFetch.preconnect })
}

/** An XMLHttpRequest that answers each upload with 201 and `body`, noting each in `sent`. */
function answerUploads(body: string, sent: Sent[]): void {
  class Uploads {
    status = 0
    responseText = ''
    withCredentials = false
    upload: { onprogress: ((event: { loaded: number }) => void) | null } = { onprogress: null }
    onload: (() => void) | null = null
    onerror: (() => void) | null = null
    onabort: (() => void) | null = null
    private method = ''
    private url = ''
    open(method: string, url: string) {
      this.method = method
      this.url = url
    }
    setRequestHeader() {}
    abort() {}
    send(sentBody: unknown) {
      sent.push({ method: this.method, url: this.url, body: sentBody })
      queueMicrotask(() => {
        this.status = 201
        this.responseText = body
        this.onload?.()
      })
    }
  }
  // The stand-in has only what an upload uses of an XMLHttpRequest, which
  // Bun does not provide; the global's type asks for all of it.
  globalThis.XMLHttpRequest = Uploads as unknown as typeof XMLHttpRequest
}

const options = () => ({ signal: new AbortController().signal, progress: () => {} })

test('a file the backend holds already is uploaded without its bytes', async () => {
  const sent: Sent[] = []
  answerFetch(() => new Response(answerBody(), { status: 201 }), sent)
  answerUploads(answerBody(), sent)
  expect(await uploadAttachment(file, options())).toEqual({
    id: 'up-1',
    mediaType: 'text/markdown',
    sha256,
    snippet: '# Notes\n',
  })
  expect(sent).toEqual([
    { method: 'POST', url: `/api/attachments?name=notes.md&sha256=${sha256}`, body: undefined },
  ])
})

test('a file the backend does not hold is sent whole, naming its hash again', async () => {
  const sent: Sent[] = []
  answerFetch(() => new Response(
    JSON.stringify({ code: 'blob_missing', message: 'Send its bytes' }),
    { status: 404 },
  ), sent)
  answerUploads(answerBody(), sent)
  expect((await uploadAttachment(file, options())).id).toBe('up-1')
  expect(sent).toEqual([
    { method: 'POST', url: `/api/attachments?name=notes.md&sha256=${sha256}`, body: undefined },
    { method: 'POST', url: `/api/attachments?name=notes.md&sha256=${sha256}`, body: file },
  ])
})
