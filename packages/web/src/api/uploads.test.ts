import { afterEach, expect, test } from 'bun:test'
import { deferred, waitFor } from '@demicodes/utils'
import { waitForBackendWith } from './client'
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

/**
 * An XMLHttpRequest that answers each upload with 201 and `body`, noting each
 * in `sent`; `away` says when one cannot reach the backend, which a proxy in
 * front of it answers with a bare 502, or which gets no answer at all.
 */
function answerUploads(body: string, sent: Sent[], away: () => 'proxy' | 'none' | null = () => null): void {
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
      const unreached = away()
      queueMicrotask(() => {
        if (unreached === 'none') {
          this.onerror?.()
          return
        }
        this.status = unreached === 'proxy' ? 502 : 201
        this.responseText = unreached === 'proxy' ? 'Bad Gateway' : body
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

// The bytes wait for the backend as every request of the page does
// (`web-application.md` § A page of another build), through the wait the
// product state registers with the HTTP client; here the test says when
// the backend is back. Cost: a few milliseconds each, no timer.

/** Registers a wait for the backend that ends when the test resolves `back`, and the undo. */
function waitForBack() {
  const back = deferred<void>()
  const waits: number[] = []
  const undo = waitForBackendWith((attempt, signal) => {
    waits.push(attempt)
    return new Promise((resolve, reject) => {
      signal.addEventListener('abort', () => reject(signal.reason), { once: true })
      void back.promise.then(resolve)
    })
  })
  return { back, waits, undo }
}

/** The backend does not hold the file: its bytes must go. */
const missing = () => new Response(JSON.stringify({ code: 'blob_missing', message: 'Send its bytes' }), { status: 404 })

test('bytes that cannot reach the backend wait for it and go again once it is back', async () => {
  const sent: Sent[] = []
  let away: 'none' | null = 'none'
  answerFetch(missing, sent)
  answerUploads(answerBody(), sent, () => away)
  const backend = waitForBack()
  try {
    const upload = uploadAttachment(file, options())
    await waitFor(() => backend.waits.length === 1, () => 'the upload did not wait')
    away = null
    backend.back.resolve()
    expect((await upload).id).toBe('up-1')
    expect(sent.filter((request) => request.body === file)).toHaveLength(2)
  } finally {
    backend.undo()
  }
})

test('an upload whose capsule goes while it waits rejects with the abort and is never sent again', async () => {
  const sent: Sent[] = []
  answerFetch(missing, sent)
  answerUploads(answerBody(), sent, () => 'proxy')
  const backend = waitForBack()
  try {
    const capsule = new AbortController()
    const upload = uploadAttachment(file, { signal: capsule.signal, progress: () => {} })
    await waitFor(() => backend.waits.length === 1, () => 'the upload did not wait')
    const removed = new Error('The capsule was removed')
    capsule.abort(removed)
    await expect(upload).rejects.toBe(removed)
    backend.back.resolve()
    await Promise.resolve()
    expect(sent.filter((request) => request.body === file)).toHaveLength(1)
  } finally {
    backend.undo()
  }
})
