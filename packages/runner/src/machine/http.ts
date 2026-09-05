// HTTP as the runner uses it: the device ends of pipes (`runner.md`
// § Pipes). A request body streams from a handle; a response body streams
// down as bytes. Proxies from the environment apply.
import * as net from 'tinyjs:net'
import { readHandle } from './stdio'

export interface HttpResponse {
  status: number
  headers: Record<string, string>
  /** The body, streamed; closes the handle when drained or dropped. */
  body: AsyncIterable<Uint8Array>
}

/** `PUT`s a readable handle as the request body; the handle is consumed by the request. */
export async function httpPut(url: string, handle: number, headers: Record<string, string>): Promise<HttpResponse> {
  return responseOf(await net.httpRequest({ method: 'PUT', url, headers, body: { handle } }))
}

export async function httpGet(url: string, headers: Record<string, string>): Promise<HttpResponse> {
  return responseOf(await net.httpRequest({ method: 'GET', url, headers }))
}

function responseOf(response: { status: number; headers: Record<string, string>; body: number }): HttpResponse {
  return { status: response.status, headers: response.headers, body: readHandle(response.body, true) }
}
