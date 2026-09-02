// The runner's end of brokered transfers (`runner.md` § Transfers): the
// backend names an origin-relative URL; this end resolves it, authenticates
// with the device token, and streams a file up or a body down.
import { httpGet, httpUploadFile, writeStreamToFile } from '@demicodes/host-runner'
import { collectBytes, decodeUtf8 } from '@demicodes/utils'

export class TransferClient {
  constructor(
    private readonly backendUrl: string,
    private readonly token: () => Promise<string | null>,
  ) {}

  /** `PUT`s the file at `path`; resolves once the backend confirmed the destination drained it. */
  async send(path: string, url: string): Promise<void> {
    const response = await httpUploadFile(this.resolve(url), path, await this.headers())
    await expectOk(response)
  }

  /** `GET`s into the file at `path`. */
  async receive(path: string, url: string): Promise<void> {
    await writeStreamToFile(path, await this.download(url))
  }

  /** `GET`s and returns the body as a stream. */
  async download(url: string): Promise<AsyncIterable<Uint8Array>> {
    const response = await httpGet(this.resolve(url), await this.headers())
    if (response.status !== 200) {
      await expectOk(response)
    }
    return response.body
  }

  private async headers(): Promise<Record<string, string>> {
    const token = await this.token()
    if (!token) throw new Error('transfer: this runner holds no device token')
    return { authorization: `Bearer ${token}` }
  }

  private resolve(url: string): string {
    return transferUrl(this.backendUrl, url)
  }
}

/** `ws(s)://host/api/runner` or `http(s)://host` plus an origin-relative path ⇒ the HTTP URL. */
export function transferUrl(backendUrl: string, path: string): string {
  const base = new URL(backendUrl)
  if (base.protocol === 'ws:') base.protocol = 'http:'
  else if (base.protocol === 'wss:') base.protocol = 'https:'
  return new URL(path, base.origin).toString()
}

async function expectOk(response: { status: number; body: AsyncIterable<Uint8Array> }): Promise<void> {
  const text = decodeUtf8(await collectBytes(response.body)).trim()
  if (response.status !== 200) throw new Error(`transfer refused (${response.status})${text ? `: ${text}` : ''}`)
}
