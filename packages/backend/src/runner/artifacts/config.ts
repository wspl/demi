import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { z } from 'zod'
import { publishPackages } from './publish'
import { S3ArtifactStore } from './s3'

const configuration = z.object({
  prefix: z.string().default('native'),
  releases: z.array(z.object({ directory: z.string().min(1), executable: z.string().regex(/^[A-Za-z0-9_-]+$/) }).strict()).min(1),
  store: z.object({
    provider: z.literal('s3'),
    bucket: z.string().min(1),
    region: z.string().min(1),
    endpoint: z.url().refine(value => new URL(value).protocol === 'https:', 'Object store endpoint requires HTTPS').optional(),
    forcePathStyle: z.boolean().default(false),
  }).strict(),
}).strict()

/** Startup publication is specific to this backend deployment. */
export async function loadNativeArtifacts(path: string, signal: AbortSignal) {
  const config = configuration.parse(JSON.parse(await readFile(path, 'utf8')))
  const releases = config.releases.map(release => ({ ...release, directory: resolve(dirname(path), release.directory) }))
  const { store: settings } = config
  const store = new S3ArtifactStore(settings.bucket, {
    region: settings.region,
    endpoint: settings.endpoint,
    forcePathStyle: settings.forcePathStyle,
  })
  try {
    const published = await publishPackages(releases, store, config.prefix, signal)
    return { ...published, close: () => store.close() }
  } catch (error) {
    store.close()
    throw error
  }
}
