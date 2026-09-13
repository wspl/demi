import { readFile } from 'node:fs/promises'
import { S3Client } from '@aws-sdk/client-s3'
import { z } from 'zod'
import { S3ChangeObjects } from './change-objects'

const configuration = z.object({
  bucket: z.string().min(1),
  region: z.string().min(1),
  endpoint: z.url({ protocol: /^https$/ }).optional(),
  forcePathStyle: z.boolean().default(false),
}).strict()

/** Deployment credentials use the AWS SDK provider chain. */
export async function loadChangeObjects(path: string) {
  const { bucket, ...options } = configuration.parse(JSON.parse(await readFile(path, 'utf8')))
  const client = new S3Client(options)
  return { objects: new S3ChangeObjects(client, bucket), close: () => client.destroy() }
}
