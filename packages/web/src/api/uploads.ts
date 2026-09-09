import { z } from 'zod'
import { ApiError, notifySessionExpired } from './client'

/** Raw bytes with actual transfer progress; every exit releases XHR listeners. */
export function uploadBytes(
  path: string,
  file: File,
  signal: AbortSignal,
  progress: (fraction: number) => void,
  mediaType = file.type || 'application/octet-stream',
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    signal.throwIfAborted()
    const xhr = new XMLHttpRequest()
    const cleanup = () => {
      xhr.onload = null
      xhr.onerror = null
      xhr.ontimeout = null
      xhr.onabort = null
      xhr.upload.onprogress = null
      signal.removeEventListener('abort', abort)
    }
    const fail = (error: Error) => {
      cleanup()
      reject(error)
    }
    const abort = () => {
      cleanup()
      xhr.abort()
      reject(signal.reason)
    }
    xhr.open('POST', `/api${path}`)
    xhr.timeout = 60_000
    xhr.withCredentials = true
    xhr.setRequestHeader('Content-Type', mediaType)
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable) {
        progress(event.loaded / event.total)
      }
    }
    xhr.onerror = () => fail(new Error('Upload connection failed'))
    xhr.ontimeout = () => fail(new Error('Upload timed out'))
    xhr.onabort = () => fail(new Error('Upload cancelled'))
    xhr.onload = () => {
      try {
        const body: unknown = JSON.parse(xhr.responseText)
        if (xhr.status < 200 || xhr.status >= 300) {
          const error = z
            .object({
              code: z.string(),
              message: z.string(),
            })
            .safeParse(body)
          if (error.success) {
            notifySessionExpired(error.data.code)
          }
          throw new ApiError(
            xhr.status,
            error.success ? error.data.code : 'upload_failed',
            error.success ? error.data.message : `Upload failed (${xhr.status})`,
          )
        }
        cleanup()
        resolve(body)
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)))
      }
    }
    signal.addEventListener('abort', abort, { once: true })
    try {
      xhr.send(file)
    } catch (error) {
      fail(error instanceof Error ? error : new Error(String(error)))
    }
  })
}
