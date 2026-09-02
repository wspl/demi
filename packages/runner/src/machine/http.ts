// HTTP as the runner uses it: brokered transfers (`runner.md` § Transfers).
// A file streams up as a request body; a response body streams down as
// bytes or straight into a file. Proxies from the environment apply.
import * as fs from 'tinyjs:fs'
import * as net from 'tinyjs:net'
import { readHandle } from './stdio'

export interface HttpResponse {
  status: number
  headers: Record<string, string>
  /** The body, streamed; closes the handle when drained or dropped. */
  body: AsyncIterable<Uint8Array>
}

/** `PUT`s the file at `path` as the request body. */
export async function httpUploadFile(url: string, path: string, headers: Record<string, string>): Promise<HttpResponse> {
  return responseOf(await net.httpRequest({ method: 'PUT', url, headers, body: { file: path } }))
}

export async function httpGet(url: string, headers: Record<string, string>): Promise<HttpResponse> {
  return responseOf(await net.httpRequest({ method: 'GET', url, headers }))
}

/** Writes a byte stream to `path`, created or truncated. */
export async function writeStreamToFile(path: string, stream: AsyncIterable<Uint8Array>, mode = 0o644): Promise<void> {
  const fd = await fs.open(path, 'w', mode)
  try {
    for await (const chunk of stream) await fs.write(fd, chunk)
  } finally {
    fs.close(fd)
  }
}

function responseOf(response: { status: number; headers: Record<string, string>; body: number }): HttpResponse {
  return { status: response.status, headers: response.headers, body: readHandle(response.body, true) }
}
