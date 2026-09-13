import type { HostFileSystem } from '@demicodes/shell'
import { decodeUtf8, encodeUtf8 } from '@demicodes/utils'
import { verifyManifest, type Manifest } from '../manifest/schema'

/**
 * Where a loader gets its manifest: in memory, a directory, a socket, a URL.
 */
export interface ManifestSource {
  manifest(): Promise<Manifest>

}

export function inMemorySource(manifest: Manifest): ManifestSource {
  return { manifest: async () => verifyManifest(manifest) }
}

/** Loads a validated immutable declaration manifest from a directory. */
export function directorySource(dir: string, fs: HostFileSystem): ManifestSource {
  return {
    manifest: async () => verifyManifest(
      JSON.parse(decodeUtf8(await fs.readFile(`${dir}/manifest.json`)))
    ),
  }
}

/** Materializes a manifest in the layout `directorySource` reads. */
export async function writeManifestDirectory(
  manifest: Manifest,
  dir: string,
  fs: HostFileSystem
): Promise<void> {
  await verifyManifest(manifest)
  await fs.mkdir(dir, { recursive: true })
  await fs.writeFile(
    `${dir}/manifest.json`,
    encodeUtf8(JSON.stringify(manifest))
  )
}
