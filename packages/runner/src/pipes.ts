// The runner's ends of pipes (`runner.md` § Pipes): the backend names an
// origin-relative URL per end; this end resolves it, authenticates with the
// device token, and `PUT`s a byte stream up or `GET`s one down. Nothing is
// held beyond what is in flight: a `PUT` body is fed through a bounded
// in-process pipe whose backpressure reaches the producer.
import * as fs from 'tinyjs:fs'
import { httpGet, httpPut, openPipe } from './machine'
import { collectBytes, decodeUtf8, noop } from '@demicodes/utils'

/** What a device end does with its URL: the shape the relay and the job table are given. */
export interface PipeEnds {
  /** `PUT`s the stream; resolves once the backend confirmed the sink drained it. */
  put(url: string, body: AsyncIterable<Uint8Array>): Promise<void>
  /** `GET`s the stream. */
  get(url: string): Promise<AsyncIterable<Uint8Array>>
}

export class PipeClient implements PipeEnds {
  constructor(
    private readonly backendUrl: string,
    private readonly token: () => Promise<string | null>,
  ) {}

  async put(url: string, body: AsyncIterable<Uint8Array>): Promise<void> {
    const headers = await this.headers()
    const { read, write } = openPipe()
    // The producer writes into the pipe as the request reads it out; a
    // request that ended (refused, or the far side gone) breaks the write,
    // which ends the producer's loop and releases its source.
    const pump = (async () => {
      try {
        for await (const chunk of body) await fs.write(write, chunk)
      } finally {
        fs.close(write)
      }
    })()
    pump.catch(noop)
    let response: Awaited<ReturnType<typeof httpPut>>
    try {
      response = await httpPut(this.resolve(url), read, headers)
    } catch (error) {
      // The request never took the handle, or failed with it: release the read end so the producer's writes break.
      try {
        fs.close(read)
      } catch {
        // Consumed by the request: already gone.
      }
      throw error
    }
    await pump.catch(noop)
    await expectOk(response)
  }

  async get(url: string): Promise<AsyncIterable<Uint8Array>> {
    const response = await httpGet(this.resolve(url), await this.headers())
    if (response.status !== 200) await expectOk(response)
    return response.body
  }

  private async headers(): Promise<Record<string, string>> {
    const token = await this.token()
    if (!token) throw new Error('pipe: this runner holds no device token')
    return { authorization: `Bearer ${token}` }
  }

  private resolve(url: string): string {
    return pipeUrl(this.backendUrl, url)
  }
}

/** `ws(s)://host/api/runner` or `http(s)://host` plus an origin-relative path ⇒ the HTTP URL. */
export function pipeUrl(backendUrl: string, path: string): string {
  const base = new URL(backendUrl)
  if (base.protocol === 'ws:') base.protocol = 'http:'
  else if (base.protocol === 'wss:') base.protocol = 'https:'
  return new URL(path, base.origin).toString()
}

async function expectOk(response: { status: number; body: AsyncIterable<Uint8Array> }): Promise<void> {
  const text = decodeUtf8(await collectBytes(response.body)).trim()
  if (response.status !== 200) throw new Error(`pipe refused (${response.status})${text ? `: ${text}` : ''}`)
}
