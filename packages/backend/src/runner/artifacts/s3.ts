import { createReadStream } from 'node:fs'
import { GetObjectCommand, HeadObjectCommand, PutObjectCommand, S3Client, type S3ClientConfig } from '@aws-sdk/client-s3'
import { getSignedUrl } from '@aws-sdk/s3-request-presigner'
import type { ArtifactStore, ObjectSource } from './store'

export class S3ArtifactStore implements ArtifactStore {
  private readonly client: S3Client

  constructor(private readonly bucket: string, options: S3ClientConfig) {
    this.client = new S3Client(options)
  }

  async putImmutable(key: string, source: ObjectSource, signal: AbortSignal): Promise<void> {
    signal.throwIfAborted()
    const body = typeof source.body === 'string' ? createReadStream(source.body) : source.body
    const checksum = Buffer.from(source.sha256, 'hex').toString('base64')
    try {
      await this.client.send(new PutObjectCommand({
        Bucket: this.bucket,
        Key: key,
        Body: body,
        ContentLength: source.size,
        ContentType: 'application/octet-stream',
        ChecksumSHA256: checksum,
        Metadata: { sha256: source.sha256 },
        IfNoneMatch: '*',
      }), { abortSignal: signal })
    } catch (error) {
      if (!(error instanceof Error && error.name === 'PreconditionFailed'))
        throw error
      const existing = await this.client.send(new HeadObjectCommand({
        Bucket: this.bucket,
        Key: key,
        ChecksumMode: 'ENABLED',
      }), { abortSignal: signal })
      if (existing.ContentLength !== source.size || existing.ChecksumSHA256 !== checksum
        || existing.Metadata?.sha256 !== source.sha256)
        throw new Error(`Immutable artifact conflicts with existing S3 object: ${key}`)
    } finally {
      if (typeof source.body === 'string' && !Buffer.isBuffer(body))
        body.destroy()
    }
  }

  async signGet(key: string, expiresIn: number, signal: AbortSignal): Promise<string> {
    signal.throwIfAborted()
    const url = await getSignedUrl(this.client, new GetObjectCommand({ Bucket: this.bucket, Key: key }), { expiresIn })
    signal.throwIfAborted()
    return url
  }

  close(): void { this.client.destroy() }
}
