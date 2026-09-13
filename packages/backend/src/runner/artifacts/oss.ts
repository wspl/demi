import { createReadStream } from 'node:fs'
import { Agent as HttpAgent } from 'node:http'
import { Agent as HttpsAgent } from 'node:https'
import OSS from 'ali-oss'
import type { ArtifactStore, ObjectSource } from './store'

export class OssArtifactStore implements ArtifactStore {
  private readonly http = new HttpAgent({ keepAlive: true, maxSockets: 2 })
  private readonly https = new HttpsAgent({ keepAlive: true, maxSockets: 2 })
  private readonly client: OSS

  constructor(options: OSS.Options) {
    const ownedOptions = { ...options, secure: true, authorizationV4: true,
      agent: this.http, httpsAgent: this.https, timeout: 60_000, retryMax: 0 }
    this.client = new OSS(ownedOptions)
  }

  async putImmutable(key: string, source: ObjectSource, signal: AbortSignal): Promise<void> {
    signal.throwIfAborted()
    // These agents serve only this publisher. Destroying them interrupts every
    // owned HTTP operation when publication is cancelled.
    const abort = () => this.close()
    signal.addEventListener('abort', abort, { once: true })
    const body = typeof source.body === 'string' ? createReadStream(source.body, { signal }) : source.body
    try {
      try {
        await this.client.put(key, body, {
          headers: { 'x-oss-forbid-overwrite': 'true', 'Content-Length': String(source.size),
            'x-oss-meta-sha256': source.sha256, 'Content-MD5': source.md5 },
        })
      } catch (error) {
        if (!(error instanceof Error && 'code' in error && error.code === 'FileAlreadyExists'))
          throw error
        const existing = await this.client.head(key)
        const headers = existing.res.headers as Record<string, unknown>
        if (Number(headers['content-length']) !== source.size
          || headers['x-oss-meta-sha256'] !== source.sha256)
          throw new Error(`Immutable artifact conflicts with existing OSS object: ${key}`)
      }
      signal.throwIfAborted()
    } finally {
      signal.removeEventListener('abort', abort)
      if (!Buffer.isBuffer(body))
        body.destroy()
    }
  }

  async signGet(key: string, expiresIn: number, signal: AbortSignal): Promise<string> {
    signal.throwIfAborted()
    const url = await this.client.signatureUrlV4('GET', expiresIn, { headers: {}, queries: {} }, key)
    signal.throwIfAborted()
    return url
  }

  close(): void {
    this.http.destroy()
    this.https.destroy()
  }
}
