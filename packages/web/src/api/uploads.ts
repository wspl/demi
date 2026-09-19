import { apiError, apiUrl } from './client'

/**
 * How long an upload may go without a byte moving before it fails, the
 * backend's own stall rule (`sessions-and-targets.md` § Host operations).
 */
const UPLOAD_STALL_MS = 60_000

/**
 * Sends a file's raw bytes, reporting how many have gone as they go. An
 * upload fails once nothing has moved for a minute, however long it takes
 * in all; the wait for the answer after the last byte counts too. Every
 * exit releases the XHR listeners and the timer. Resolves with the answer's
 * JSON, or null for an answer without a body.
 */
export function uploadBytes(
  path: string,
  file: File,
  options: {
    method?: 'POST' | 'PUT'
    mediaType?: string
    signal: AbortSignal
    progress: (sent: number) => void
  },
): Promise<unknown> {
  const { signal } = options
  return new Promise((resolve, reject) => {
    signal.throwIfAborted()
    const xhr = new XMLHttpRequest()
    let stall: ReturnType<typeof setTimeout> | undefined
    const cleanup = () => {
      xhr.onload = null
      xhr.onerror = null
      xhr.onabort = null
      xhr.upload.onprogress = null
      clearTimeout(stall)
      signal.removeEventListener('abort', abort)
    }
    const fail = (error: Error) => {
      cleanup()
      reject(error)
    }
    const stop = (reason: unknown) => {
      cleanup()
      xhr.abort()
      reject(reason)
    }
    const abort = () => stop(signal.reason)
    const moved = () => {
      clearTimeout(stall)
      stall = setTimeout(() => stop(new Error('Upload stalled')), UPLOAD_STALL_MS)
    }
    xhr.open(options.method ?? 'POST', apiUrl(path))
    xhr.withCredentials = true
    xhr.setRequestHeader('Content-Type', options.mediaType ?? (file.type || 'application/octet-stream'))
    xhr.upload.onprogress = (event) => {
      moved()
      options.progress(event.loaded)
    }
    xhr.onerror = () => fail(new Error('Upload connection failed'))
    xhr.onabort = () => fail(new Error('Upload cancelled'))
    xhr.onload = () => {
      if (xhr.status < 200 || xhr.status >= 300) {
        fail(apiError(xhr.status, xhr.responseText))
        return
      }
      try {
        const body: unknown = xhr.responseText ? JSON.parse(xhr.responseText) : null
        cleanup()
        resolve(body)
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)))
      }
    }
    signal.addEventListener('abort', abort, { once: true })
    moved()
    try {
      xhr.send(file)
    } catch (error) {
      fail(error instanceof Error ? error : new Error(String(error)))
    }
  })
}
