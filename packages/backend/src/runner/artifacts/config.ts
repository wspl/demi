import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { z } from 'zod'
import { OssArtifactStore } from './oss'
import { publishPackages } from './publish'
import { S3ArtifactStore } from './s3'

const storeFields = {
  bucket: z.string().min(1),
  region: z.string().min(1),
  endpoint: z.url().refine(value => new URL(value).protocol === 'https:', 'Object store endpoint requires HTTPS').optional(),
}
const configuration = z.object({
  prefix: z.string().default('native'),
  releases: z.array(z.object({ directory: z.string().min(1), executable: z.string().regex(/^[A-Za-z0-9_-]+$/) }).strict()).min(1),
  store: z.discriminatedUnion('provider', [
    z.object({ provider: z.literal('s3'), ...storeFields, forcePathStyle: z.boolean().default(false) }).strict(),
    z.object({ provider: z.literal('oss'), ...storeFields }).strict(),
  ]),
}).strict()

function credential(name: string): string {
  const value = process.env[name]
  if (!value)
    throw new Error(`Missing object storage credential environment variable: ${name}`)
  return value
}

/** Startup publication is specific to this backend deployment. */
export async function loadNativeArtifacts(path: string, signal: AbortSignal) {
  const config = configuration.parse(JSON.parse(await readFile(path, 'utf8')))
  const releases = config.releases.map(release => ({ ...release, directory: resolve(dirname(path), release.directory) }))
  const { store: settings } = config
  const store = settings.provider === 's3'
    ? new S3ArtifactStore(settings.bucket, { region: settings.region, endpoint: settings.endpoint,
      forcePathStyle: settings.forcePathStyle })
    : new OssArtifactStore({ bucket: settings.bucket, region: settings.region, endpoint: settings.endpoint,
      accessKeyId: credential('ALIBABA_CLOUD_ACCESS_KEY_ID'),
      accessKeySecret: credential('ALIBABA_CLOUD_ACCESS_KEY_SECRET'),
      stsToken: process.env.ALIBABA_CLOUD_SECURITY_TOKEN })
  try {
    const published = await publishPackages(releases, store, config.prefix, signal)
    return { ...published, close: () => store.close() }
  } catch (error) {
    store.close()
    throw error
  }
}
